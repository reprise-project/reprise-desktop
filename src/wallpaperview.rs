//! The desktop wallpaper (`[wallpaper]`, and an output's own `[[outputs]] wallpaper`), decoded
//! (a built-in is drawn at each output's own size) off the loop and composed per output into
//! `layers.background`, and the slideshow: every `interval` minutes a thread picks the folder's
//! next picture and the new buffers fade in over the old.
//!
//! An output with a picture of its own shows it on every desktop, in place of the desktop's own
//! picture and the slideshow; the others show the current desktop's. Outputs placing the same file
//! alike share one decode, and a change carries over the pictures and buffers of the outputs whose
//! picture stays, so only the outputs that change fade.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reprisekit::wallpapers::Builtin;
use reprisescene::BufferId;
use smithay::{
    backend::renderer::gles::GlesTexture,
    output::Output,
    reexports::calloop::{
        RegistrationToken, channel,
        timer::{TimeoutAction, Timer},
    },
};

use crate::{
    compositor::State,
    config::WallpaperConfig,
    dmaupload::{Staged, Uploads},
    output::upload_scaled,
    raster::{Canvas, Rect},
    theme::Color,
    wallpaper::{Cut, Drawn, Loaded, Position},
    waterview::Pane,
};

/// Which picture each output shows: its own, else `shared`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Paths {
    pub shared: Option<PathBuf>,
    /// By connector name, as `[[outputs]]` names them.
    pub own: Vec<(String, PathBuf)>,
}

impl Paths {
    /// `path` on every output.
    pub fn one(path: Option<&Path>) -> Paths {
        Paths {
            shared: path.map(Path::to_path_buf),
            own: Vec::new(),
        }
    }

    /// `shared` on every output without a picture of its own in `config`.
    pub fn with_own(shared: Option<PathBuf>, config: &WallpaperConfig) -> Paths {
        let own = (config.outputs.iter())
            .filter_map(|o| Some((o.name.clone(), o.image.clone()?)))
            .collect();
        Paths { shared, own }
    }

    pub fn of(&self, output: &str) -> Option<&Path> {
        (self.own.iter())
            .find(|(n, _)| n == output)
            .map(|(_, p)| p.as_path())
            .or(self.shared.as_deref())
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.shared.is_none() && self.own.is_empty()
    }

    fn all(&self) -> impl Iterator<Item = &Path> {
        (self.shared.iter().map(PathBuf::as_path)).chain(self.own.iter().map(|(_, p)| p.as_path()))
    }
}

/// The position a file at `path` is reduced and placed for, None for a built-in: drawn at each
/// output's size, it fills the screen whatever the position.
fn placing(path: &Path, configured: Position) -> Option<Position> {
    crate::wallpaper::builtin(path)
        .is_none()
        .then_some(configured)
}

/// How an output places `path`: Stretch for a built-in, as it is drawn at the output's size.
fn placed(path: Option<&Path>, configured: Position) -> Position {
    path.and_then(|p| placing(p, configured))
        .unwrap_or(Position::Stretch)
}

/// One picture the outputs show: a file decoded once for every output placing it alike, or a
/// built-in and its drawings, one per output device size.
struct Picture {
    path: PathBuf,
    /// What a file is reduced for; None for a built-in.
    position: Option<Position>,
    /// A file's picture; None until the first decode lands, and for a built-in. Shared with the
    /// jobs composing it.
    image: Option<Arc<Loaded>>,
    /// The cut a decode in flight is making; a finished decode for any other is stale and dropped.
    loading: Option<Cut>,
    builtin: Option<(Builtin, Drawn)>,
    /// Built-in pictures made ready for the scene on the drawing thread, one per size an output
    /// shows; every refit of that size shows it again.
    staged: Vec<Staged>,
    /// The file did not decode: its outputs show the desktop colour.
    failed: bool,
}

impl Picture {
    fn new(path: &Path, position: Option<Position>) -> Picture {
        Picture {
            path: path.to_path_buf(),
            position,
            image: None,
            loading: None,
            builtin: crate::wallpaper::builtin(path).map(|b| (b, Drawn::default())),
            staged: Vec::new(),
            failed: false,
        }
    }

    fn is(&self, path: &Path, position: Option<Position>) -> bool {
        self.path == path && self.position == position
    }

    /// What it holds, for a wallpaper whose jobs start over: nothing in flight.
    fn carried(mut self) -> Picture {
        self.loading = None;
        if let Some((_, d)) = &mut self.builtin {
            d.forget_drawing();
        }
        self
    }

    fn shows_something(&self) -> bool {
        self.image.is_some()
            || self
                .builtin
                .as_ref()
                .is_some_and(|(_, d)| d.any().is_some())
    }

    /// The picture an output of `size` device pixels is composed from: a built-in's own picture
    /// of that size, else the file's.
    fn source(&self, size: (u32, u32)) -> Option<&Canvas> {
        match &self.builtin {
            Some((_, d)) => d.get(size).map(|p| &**p),
            None => self.image.as_ref().map(|l| &l.image),
        }
    }
}

/// The wallpaper and its scene buffers, one per output.
pub struct Wallpaper {
    pub paths: Paths,
    /// One per picture the outputs show, kept from the wallpaper before where it shows there too.
    pictures: Vec<Picture>,
    nodes: Vec<(Output, WallpaperNode)>,
    /// The pictures this one replaces, still on screen under it until this one has faded in:
    /// their buffers, and the fade's start once it runs.
    fading: Vec<BufferId>,
    fade: Option<Instant>,
    /// How long this picture takes to fade in over the one before.
    fade_for: Duration,
    /// Its number in [`Jobs`]; a decode or drawing for a wallpaper since replaced is skipped.
    serial: u64,
    /// An output waits for its picture at the last refit, which holds back the fade.
    unready: bool,
    /// The Alt+Tab desktop entry's last picture and what it was composed for; a picture
    /// landing drops it.
    still: Option<(StillKey, Rc<Canvas>)>,
}

/// A wallpaper picture shared with a job composing from it.
#[derive(Clone)]
enum Shared {
    Drawn(Arc<Canvas>),
    Image(Arc<Loaded>),
}

impl Shared {
    fn canvas(&self) -> &Canvas {
        match self {
            Shared::Drawn(c) => c,
            Shared::Image(l) => &l.image,
        }
    }

    fn same(&self, other: &Shared) -> bool {
        match (self, other) {
            (Shared::Drawn(a), Shared::Drawn(b)) => Arc::ptr_eq(a, b),
            (Shared::Image(a), Shared::Image(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// What a desktop entry's still depends on besides the wallpaper's own pictures.
#[derive(PartialEq)]
struct StillKey {
    out: Option<Output>,
    size: (u32, u32),
    layout: Rect,
    scale: i32,
    position: Position,
    bg: Color,
}

impl Wallpaper {
    pub fn new(paths: Paths) -> Wallpaper {
        Wallpaper {
            paths,
            pictures: Vec::new(),
            nodes: Vec::new(),
            fading: Vec::new(),
            fade: None,
            fade_for: FADE,
            serial: 0,
            unready: false,
            still: None,
        }
    }

    /// The picture output `name` shows placed at `position`.
    fn picture_at(&self, name: &str, position: Position) -> Option<&Picture> {
        let path = self.paths.of(name)?;
        let key = placing(path, position);
        self.pictures.iter().find(|p| p.is(path, key))
    }

    fn picture_mut(&mut self, path: &Path, position: Option<Position>) -> Option<&mut Picture> {
        self.pictures.iter_mut().find(|p| p.is(path, position))
    }

    fn land_image(&mut self, path: &Path, position: Option<Position>, loaded: Loaded) {
        if let Some(p) = self.picture_mut(path, position) {
            p.image = Some(Arc::new(loaded));
            self.still = None;
        }
    }

    /// Built-in `path`'s picture lands; false (and dropped) when no output wants its size any more.
    fn land_drawn(&mut self, path: &Path, picture: Canvas) -> bool {
        let landed = (self.picture_mut(path, None))
            .and_then(|p| p.builtin.as_mut())
            .is_some_and(|(_, d)| d.land(picture));
        if landed {
            self.still = None;
        }
        landed
    }

    /// The still for `key`: the last one when it was composed for the same key, else `compose`'s.
    fn still(&mut self, key: StillKey, compose: impl FnOnce(&Wallpaper) -> Canvas) -> Rc<Canvas> {
        match &self.still {
            Some((k, c)) if *k == key => c.clone(),
            _ => {
                let c = Rc::new(compose(self));
                self.still = Some((key, c.clone()));
                c
            }
        }
    }

    /// Whether output `name` shows the desktop colour alone: no picture of its own or shared, none
    /// decoded or drawn yet, and none fading out.
    pub(crate) fn shows_nothing_on(&self, name: &str, position: Position) -> bool {
        self.fading.is_empty()
            && self
                .picture_at(name, position)
                .is_none_or(|p| p.failed || !p.shows_something())
    }

    /// The picture output `name` of `size` device pixels is composed from.
    fn source(&self, name: &str, position: Position, size: (u32, u32)) -> Option<&Canvas> {
        self.picture_at(name, position)?.source(size)
    }

    /// The picture the still of `key` is composed from: a built-in's of the output's `size`
    /// (any for the whole layout, the shared picture's), else the file's.
    fn still_source(&self, key: &StillKey, size: (u32, u32)) -> Option<Shared> {
        let p = match &key.out {
            Some(o) => self.picture_at(&o.name(), key.position)?,
            None => {
                let path = self.paths.all().next()?;
                let at = placing(path, key.position);
                self.pictures.iter().find(|p| p.is(path, at))?
            }
        };
        match &p.builtin {
            Some((_, d)) if key.out.is_none() => d.any().cloned().map(Shared::Drawn),
            Some((_, d)) => d.get(size).cloned().map(Shared::Drawn),
            None => p.image.clone().map(Shared::Image),
        }
    }

    /// A still composed on a job from `src` lands: kept unless a picture landed meanwhile.
    fn land_still(&mut self, key: StillKey, size: (u32, u32), src: &Shared, still: Canvas) {
        if self.still_source(&key, size).is_some_and(|s| s.same(src)) {
            self.still = Some((key, Rc::new(still)));
        }
    }

    /// Whether the fade from the picture before is due: none runs yet, no output waits for its
    /// picture and none is still composing.
    fn fade_due(&self) -> bool {
        !self.fading.is_empty()
            && self.fade.is_none()
            && !self.unready
            && self.nodes.iter().all(|(_, n)| n.composing.is_none())
    }

    /// Where the lock screen's backdrop on `out`, at the output's current `fit`, comes from.
    fn lock_source(&self, out: &Output, fit: Fit) -> LockSource {
        let node = self.nodes.iter().find(|(o, _)| o == out).map(|(_, n)| n);
        let file = self
            .picture_at(&out.name(), fit.position)
            .is_some_and(|p| p.image.is_some());
        match node {
            Some(n) if n.fit == Some(fit) => match &n.texture {
                Some(t) => LockSource::Shown(t.clone()),
                None => LockSource::Buffer(n.buffer),
            },
            // A refit composes it on a job, whose landing repaints the lock screen.
            _ if file => LockSource::Composing,
            _ => LockSource::Here,
        }
    }

    pub(crate) fn buffers(&self) -> (Vec<(Output, BufferId)>, Vec<BufferId>) {
        let shown = self
            .nodes
            .iter()
            .map(|(o, n)| (o.clone(), n.buffer))
            .collect();
        (shown, self.fading.clone())
    }
}

enum LockSource {
    Shown(GlesTexture),
    Buffer(BufferId),
    Composing,
    Here,
}

/// The lock screen's backdrop on an output.
pub enum LockWall {
    /// The staged texture the desktop shows on the output.
    Shown(GlesTexture),
    /// The desktop's buffer on the output, whose pixels are in memory: a built-in, or a picture
    /// staged in pixel memory.
    Shared(BufferId),
    /// A job is composing the picture; until it lands the lock screen keeps what it shows.
    Composing,
    /// Composed on the loop: a built-in the desktop shows at another fit.
    Composed(Canvas),
    None,
}

/// How long a slideshow picture takes to fade into the next.
const FADE: Duration = Duration::from_millis(1200);
/// A desktop switch's fade to the next desktop's picture when no switch animation is set.
const DESKTOP_FADE: Duration = Duration::from_millis(250);
/// How long after a picture lands Alt+Tab's desktop still is composed: past `FADE`.
const STILL_WARM_DELAY: Duration = Duration::from_secs(2);

/// The wallpaper's decodes and drawings, run one at a time: switching desktops through several
/// pictures holds the one showing, the one fading out and at most one in the making. A decode in
/// reprise-decode is stopped once a newer wallpaper is set, so it never holds up that one.
#[derive(Default)]
pub struct Jobs {
    /// The serial of the wallpaper set last.
    latest: AtomicU64,
    one: Mutex<()>,
}

impl Jobs {
    fn next(&self) -> u64 {
        self.latest.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// Whether a wallpaper newer than `serial` was set.
    fn replaced(&self, serial: u64) -> bool {
        self.latest.load(Ordering::Acquire) != serial
    }

    /// `f`, once no other job runs, unless wallpaper `serial` was replaced by then.
    fn run<T>(&self, serial: u64, f: impl FnOnce() -> T) -> Option<T> {
        let _one = self.one.lock().unwrap_or_else(|e| e.into_inner());
        (!self.replaced(serial)).then(f)
    }
}

/// The running `[wallpaper] slideshow`: its folder, the timer for the
/// next change, and whether a pick is on its way from the thread.
pub struct Slideshow {
    folder: PathBuf,
    timer: Option<RegistrationToken>,
    picking: bool,
    /// The picture last picked, which every desktop without its own shows.
    shown: Option<PathBuf>,
}

struct WallpaperNode {
    buffer: BufferId,
    fit: Option<Fit>,
    /// The logical box the buffer covers; None until it first holds a picture.
    placed: Option<Rect>,
    /// The fit a job is composing for this output.
    composing: Option<Fit>,
    /// The staged dma-buf's texture the buffer shows, which the desktop water samples too.
    texture: Option<GlesTexture>,
    /// Made over a picture fading out: it waits unseen for the fade, which raises it.
    fades: bool,
}

impl WallpaperNode {
    /// Stretch the buffer over `b`; true when it moved.
    fn place(&mut self, scene: &mut crate::compositor::DeskScene, b: Rect) -> bool {
        if self.placed.is_none_or(|p| p == b) {
            return false;
        }
        scene.set_position(self.buffer, b.x, b.y);
        scene.buffer_set_dest_size(self.buffer, Some((b.width, b.height).into()));
        self.placed = Some(b);
        true
    }

    /// The buffer holds the picture composed for `fit`; a job composing another fit lands stale.
    fn shown(&mut self, fit: Fit, texture: Option<GlesTexture>) {
        self.fit = Some(fit);
        self.texture = texture;
        self.placed = Some(fit.rect);
        self.composing = None;
    }

    /// Whether a job's picture composed for `fit` is still wanted.
    fn takes(&self, fit: Fit) -> bool {
        self.composing == Some(fit)
    }
}

/// The staged picture of `size`, kept for the other outputs and later refits of that size.
fn staged_of(staged: &[Staged], size: (u32, u32)) -> Option<Staged> {
    staged.iter().find(|p| staged_size(p) == size).cloned()
}

fn staged_size(p: &Staged) -> (u32, u32) {
    (p.size().0 as u32, p.size().1 as u32)
}

/// What an output's wallpaper is composed from besides the image; the same
/// again leaves the buffer as it is.
#[derive(Clone, Copy, PartialEq)]
struct Fit {
    rect: Rect,
    scale: i32,
    position: Position,
    bg: Color,
}

/// A picture's path and what a file is reduced for ([`placing`]).
type PictureKey = (PathBuf, Option<Position>);

/// One output's part in a refit.
struct Shown {
    out: Output,
    rect: Rect,
    scale: i32,
    size: (u32, u32),
    /// Its picture in [`Wallpaper::pictures`]; None for the desktop colour alone.
    picture: Option<usize>,
    position: Position,
}

impl State {
    /// Put `path` (or nothing) under the desktop icons on every output.
    pub fn set_wallpaper(&mut self, path: Option<&Path>) {
        self.set_wallpapers(Paths::one(path));
    }

    /// Show `paths`. A change to built-ins alone fades in over the pictures before (a new glass
    /// colour retints Glass's), which stay until drawn; any other cuts to the desktop colour on the
    /// outputs that change until their pictures land.
    fn set_wallpapers(&mut self, paths: Paths) {
        match &self.wallpaper {
            Some(w) if w.paths == paths => self.refit_wallpaper(),
            Some(w) => {
                let drawn = (paths.all())
                    .filter(|p| !w.paths.all().any(|q| q == *p))
                    .all(|p| crate::wallpaper::builtin(p).is_some());
                self.change_wallpaper(paths, drawn.then_some(FADE));
            }
            None => self.change_wallpaper(paths, None),
        }
    }

    fn new_wallpaper(&self, paths: Paths) -> Wallpaper {
        Wallpaper {
            serial: self.wallpaper_jobs.next(),
            ..Wallpaper::new(paths)
        }
    }

    /// Replace the wallpaper with `paths`: over `fade` the outputs whose picture changes fade
    /// from the one before, which stays under until then; with none they show the desktop colour
    /// until theirs lands. Outputs whose picture stays keep their buffers, and pictures still shown
    /// are carried over undecoded again.
    fn change_wallpaper(&mut self, paths: Paths, fade: Option<Duration>) {
        if paths.is_empty() {
            self.clear_wallpaper();
            return;
        }
        let mut next = self.new_wallpaper(paths);
        if let Some(d) = fade {
            next.fade_for = match self.config.effects.reduce_motion {
                true => Duration::ZERO,
                false => d,
            };
        }
        let mut cut = false;
        if let Some(old) = self.wallpaper.take() {
            next.pictures = (old.pictures.into_iter())
                .filter(|p| !p.failed)
                .map(Picture::carried)
                .collect();
            // A fade cut short leaves its pictures whole; a picture waiting unseen over the one
            // before for its fade to start is dropped, and the one before stays.
            let running = old.fade.is_some() || fade.is_none();
            for (o, mut n) in old.nodes {
                if running && n.fades {
                    self.scene.set_opacity(n.buffer, 1.0);
                    n.fades = false;
                }
                let name = o.name();
                if old.paths.of(&name) == next.paths.of(&name) {
                    next.nodes.push((o, n));
                } else if fade.is_some() && !n.fades {
                    self.scene.set_opacity(n.buffer, 1.0);
                    next.fading.push(n.buffer);
                } else {
                    self.scene.destroy(n.buffer);
                    cut = true;
                }
            }
            if running {
                cut |= !old.fading.is_empty();
                self.end_fade(old.fading);
            } else {
                next.fading.extend(old.fading);
            }
        }
        self.wallpaper = Some(next);
        if cut {
            crate::lockview::refit(self);
            self.redraw_all();
        }
        self.refit_wallpaper();
    }

    /// Decode wallpaper picture `path`, reduced for `position`, cut to `cut` on a thread; it lands
    /// in [`State::wallpaper_loaded`].
    fn load_wallpaper(&mut self, path: PathBuf, position: Option<Position>, cut: Cut) {
        let Some(serial) = self.wallpaper.as_ref().map(|w| w.serial) else {
            return;
        };
        let (tx, rx) = channel::channel::<Option<Loaded>>();
        let at = path.clone();
        let inserted = self.handle.insert_source(rx, move |event, _, state| {
            if let channel::Event::Msg(loaded) = event {
                state.wallpaper_loaded(serial, &at, position, cut, loaded);
            }
        });
        if let Err(e) = inserted {
            log::warn!("wallpaper: {}", e.error);
            return;
        }
        let jobs = self.wallpaper_jobs.clone();
        let spawned = std::thread::Builder::new()
            .name("wallpaper".into())
            .spawn(move || {
                // A skipped decode sends nothing: the wallpaper that wanted it is gone.
                let wanted = crate::wallpaper::Wanted {
                    deadline: reprisekit::wallpaper::DECODE_DEADLINE,
                    stop: &|| jobs.replaced(serial),
                };
                if let Some(loaded) =
                    jobs.run(serial, || crate::wallpaper::load(&path, Some(cut), wanted))
                {
                    let _ = tx.send(loaded);
                }
            });
        if let Err(e) = spawned {
            log::warn!("wallpaper: no decode thread: {e}");
        }
    }

    /// Wallpaper `serial`'s decode of picture `path` cut to `cut` lands: it replaces the picture
    /// and recomposes its outputs. A file that failed to decode leaves its outputs the desktop
    /// colour, and no wallpaper once every picture failed. A decode for another cut, or for a
    /// wallpaper since replaced (even by the same file), is dropped: a stopped one lands as None.
    fn wallpaper_loaded(
        &mut self,
        serial: u64,
        path: &Path,
        position: Option<Position>,
        cut: Cut,
        loaded: Option<Loaded>,
    ) {
        let Some(w) = self.wallpaper.as_mut() else {
            return;
        };
        if w.serial != serial {
            return;
        }
        let Some(p) = w.picture_mut(path, position) else {
            return;
        };
        if p.loading != Some(cut) {
            return;
        }
        p.loading = None;
        match loaded {
            Some(l) => {
                w.land_image(path, position, l);
                let mine: Vec<Output> = (w.nodes.iter())
                    .filter(|(o, _)| w.paths.of(&o.name()) == Some(path))
                    .map(|(o, _)| o.clone())
                    .collect();
                for (o, node) in &mut w.nodes {
                    if mine.contains(o) {
                        node.fit = None;
                        node.composing = None;
                    }
                }
                self.refit_wallpaper();
                crate::lockview::refit(self);
                self.warm_switcher_still_soon();
            }
            None => {
                p.failed = true;
                if w.pictures.iter().all(|p| p.failed) {
                    self.clear_wallpaper();
                } else {
                    self.refit_wallpaper();
                    crate::lockview::refit(self);
                }
                crate::greeterview::background_failed(self);
            }
        }
    }

    /// A picture landed: compose Alt+Tab's desktop still a moment later,
    /// away from the frames that show the new picture.
    fn warm_switcher_still_soon(&mut self) {
        let timer = Timer::from_duration(STILL_WARM_DELAY);
        let inserted = self.handle.insert_source(timer, |_, _, state| {
            state.warm_switcher_still();
            TimeoutAction::Drop
        });
        if let Err(e) = inserted {
            log::warn!("wallpaper: {}", e.error);
        }
    }

    /// Draw built-in `b` at each of `sizes` on a thread, one after the
    /// other. Each picture lands on the loop as it is done, and the
    /// outputs of its size compose it, unless the wallpaper changed or no
    /// output wants that size any more.
    fn draw_builtins(&mut self, path: PathBuf, b: Builtin, sizes: Vec<(u32, u32)>) {
        let (tx, rx) = channel::channel::<(Canvas, Staged)>();
        let inserted = self.handle.insert_source(rx, move |event, _, state| {
            let channel::Event::Msg((picture, staged)) = event else {
                return;
            };
            let Some(w) = state.wallpaper.as_mut() else {
                return;
            };
            if w.land_drawn(&path, picture) {
                if let Some(p) = w.picture_mut(&path, None) {
                    p.staged.push(staged);
                }
                state.refit_wallpaper();
                crate::lockview::refit(state);
                state.warm_switcher_still_soon();
            }
        });
        if let Err(e) = inserted {
            log::warn!("wallpaper: {}", e.error);
            return;
        }
        let uploads = self.uploads();
        let serial = self.wallpaper.as_ref().map_or(0, |w| w.serial);
        let jobs = self.wallpaper_jobs.clone();
        let spawned = std::thread::Builder::new()
            .name("wallpaper".into())
            .spawn(move || {
                for (w, h) in sizes {
                    let Some(picture) = jobs.run(serial, || crate::wallpaper::draw(b, w, h)) else {
                        return;
                    };
                    let staged = Staged::new(uploads.as_ref(), &picture);
                    if tx.send((picture, staged)).is_err() {
                        return;
                    }
                }
            });
        if let Err(e) = spawned {
            log::warn!("wallpaper: no drawing thread: {e}");
        }
    }

    fn clear_wallpaper(&mut self) {
        if let Some(w) = self.wallpaper.take() {
            for (_, node) in w.nodes {
                self.scene.destroy(node.buffer);
            }
            self.end_fade(w.fading);
            self.water_configured();
            crate::lockview::refit(self);
            self.redraw_all();
        }
    }

    fn end_fade(&mut self, fading: Vec<BufferId>) {
        for b in fading {
            self.scene.destroy(b);
        }
    }

    fn fade_wallpapers_over(&mut self, paths: Paths, fade_for: Duration) {
        match &self.wallpaper {
            Some(w) if w.paths == paths => {}
            Some(_) => self.change_wallpaper(paths, Some(fade_for)),
            None => self.set_wallpapers(paths),
        }
    }

    fn start_fade(&mut self) {
        if let Some(w) = self.wallpaper.as_mut() {
            w.fade = Some(Instant::now());
            self.animate(crate::animclock::Anim::Wallpaper);
        }
    }

    /// One frame of the fade from the picture before; whether it still runs.
    pub(crate) fn wallpaper_fade_step(&mut self, now: Instant) -> bool {
        let Some(w) = self.wallpaper.as_mut() else {
            return false;
        };
        let Some(at) = w.fade else {
            return false;
        };
        let since = now.saturating_duration_since(at);
        let t = (since.as_secs_f32() / w.fade_for.as_secs_f32().max(0.001)).min(1.0);
        let done = t >= 1.0;
        for (_, n) in &mut w.nodes {
            if n.fades {
                self.scene.set_opacity(n.buffer, t);
                n.fades = !done;
            }
        }
        let fading = if done {
            w.fade = None;
            std::mem::take(&mut w.fading)
        } else {
            Vec::new()
        };
        self.water_fade(t);
        for b in fading {
            self.scene.destroy(b);
        }
        self.redraw_all();
        !done
    }

    /// Follow `[wallpaper]`'s image and slideshow keys and the outputs' own pictures after a
    /// reload, and at startup: with a slideshow its folder's pictures take turns (a new folder
    /// picks at once), else `image` shows.
    pub fn apply_wallpaper_config(&mut self) {
        let Some(folder) = self.config.wallpaper.slideshow.clone() else {
            if let Some(t) = self.slideshow.take().and_then(|s| s.timer) {
                self.handle.remove(t);
            }
            let paper = (self.config.wallpaper)
                .for_desktop(self.desktops.current)
                .map(Path::to_path_buf);
            self.set_wallpapers(Paths::with_own(paper, &self.config.wallpaper));
            return;
        };
        if self.slideshow.as_ref().is_some_and(|s| s.folder == folder) {
            // The interval may have changed: count it from now.
            self.arm_slideshow();
            self.follow_desktop_wallpaper(FADE);
            return;
        }
        if let Some(t) = self.slideshow.take().and_then(|s| s.timer) {
            self.handle.remove(t);
        }
        self.slideshow = Some(Slideshow {
            folder,
            timer: None,
            picking: false,
            shown: None,
        });
        self.follow_desktop_wallpaper(FADE);
        self.next_slide();
    }

    /// What the outputs show on the current desktop: their own pictures, and on the rest the
    /// desktop's own, else the slideshow's pick, else `[wallpaper] image`. None while a slideshow
    /// has picked nothing yet.
    fn desktop_wallpaper(&self) -> Option<Paths> {
        let own = self.config.wallpaper.desktop(self.desktops.current);
        let shared = match (own, &self.slideshow) {
            (Some(p), _) => Some(p.to_path_buf()),
            (None, Some(show)) => Some(show.shown.clone()?),
            (None, None) => self.config.wallpaper.image.clone(),
        };
        Some(Paths::with_own(shared, &self.config.wallpaper))
    }

    /// Fade to the current desktop's pictures over `fade_for` where they are not the ones showing.
    fn follow_desktop_wallpaper(&mut self, fade_for: Duration) {
        if let Some(want) = self.desktop_wallpaper() {
            self.fade_wallpapers_over(want, fade_for);
        }
    }

    /// A desktop switch: the new desktop's picture fades in over the switch animation's
    /// duration, or [`DESKTOP_FADE`] without one.
    pub fn desktop_wallpaper_switched(&mut self) {
        let a = self.config.workspace_animation;
        let fade_for = match a.enabled() {
            true => Duration::from_millis(a.ms.into()),
            false => DESKTOP_FADE,
        };
        self.follow_desktop_wallpaper(fade_for);
    }

    /// Whether `out` shows its wallpaper, or has none to wait for.
    pub fn wallpaper_ready(&self, out: &Output) -> bool {
        if self.slideshow.as_ref().is_some_and(|s| s.picking) {
            return false;
        }
        let name = out.name();
        let position = self.config.wallpaper.position_for(&name);
        self.wallpaper.as_ref().is_none_or(|w| {
            w.nodes.iter().any(|(o, n)| o == out && n.fit.is_some())
                || w.paths.of(&name).is_none()
                || w.picture_at(&name, position).is_some_and(|p| p.failed)
        })
    }

    /// Whether a slideshow runs (the desktop menu's "Next desktop background").
    pub fn slideshow_running(&self) -> bool {
        self.slideshow.is_some()
    }

    /// Pick the slideshow's next picture on a thread and fade to it; the
    /// interval counts again from then. No-op without a slideshow or
    /// while a pick is under way.
    pub fn next_slide(&mut self) {
        let shuffle = self.config.wallpaper.shuffle;
        let Some(show) = self.slideshow.as_mut() else {
            return;
        };
        let current = show.shown.clone();
        if show.picking {
            return;
        }
        show.picking = true;
        let folder = show.folder.clone();
        let (tx, rx) = channel::channel::<Option<PathBuf>>();
        let inserted = self.handle.insert_source(rx, move |event, _, state| {
            let channel::Event::Msg(pick) = event else {
                return;
            };
            let Some(show) = state.slideshow.as_mut() else {
                return;
            };
            show.picking = false;
            match pick {
                Some(p) => {
                    show.shown = Some(p);
                    state.follow_desktop_wallpaper(FADE);
                }
                None => log::info!("slideshow: no pictures in {}", show.folder.display()),
            }
            state.arm_slideshow();
        });
        if let Err(e) = inserted {
            log::warn!("slideshow: {}", e.error);
            return;
        }
        let spawned = std::thread::Builder::new()
            .name("slideshow".into())
            .spawn(move || {
                let list = reprisefs::pictures::in_folder(&folder);
                let roll = reprisefs::pictures::roll();
                let _ = tx.send(reprisefs::pictures::next(
                    &list,
                    current.as_deref(),
                    shuffle,
                    roll,
                ));
            });
        if let Err(e) = spawned {
            log::warn!("slideshow: no thread: {e}");
        }
    }

    fn arm_slideshow(&mut self) {
        let minutes = self.config.wallpaper.interval.max(1) as u64;
        let Some(show) = self.slideshow.as_mut() else {
            return;
        };
        if let Some(t) = show.timer.take() {
            self.handle.remove(t);
        }
        let timer = Timer::from_duration(Duration::from_secs(minutes * 60));
        match self.handle.insert_source(timer, |_, _, state| {
            if let Some(s) = state.slideshow.as_mut() {
                s.timer = None;
            }
            state.next_slide();
            TimeoutAction::Drop
        }) {
            Ok(t) => {
                if let Some(show) = self.slideshow.as_mut() {
                    show.timer = Some(t);
                }
            }
            Err(e) => log::warn!("slideshow: {}", e.error),
        }
    }

    /// How output `name` places its picture under `w`.
    fn placement(&self, w: &Wallpaper, name: &str) -> Position {
        placed(w.paths.of(name), self.config.wallpaper.position_for(name))
    }

    /// Compose the wallpaper for every output's current size and position; an output whose fit has
    /// not changed keeps its buffer. Each picture an output shows is decoded (or a built-in drawn
    /// at each of its outputs' sizes not yet drawn) once for all of them, and pictures and sizes no
    /// output shows are dropped.
    pub fn refit_wallpaper(&mut self) {
        let outputs: Vec<Output> = self.own_outputs();
        let bg = self.config.theme.colors.background;
        // Taken out for the loop so the pictures are borrowed, not cloned,
        // while the state is asked for boxes and scales; put back below.
        let Some(mut w) = self.wallpaper.take() else {
            self.water_configured();
            return;
        };
        w.nodes.retain(|(o, node)| {
            let alive = outputs.contains(o);
            if !alive {
                self.scene.destroy(node.buffer);
            }
            alive
        });
        let mut shown: Vec<(Shown, Option<PictureKey>)> = Vec::new();
        for out in outputs {
            let rect = self.layout_box(Some(&out));
            let scale = self.draw_scale_at(Some(&out));
            let size = ((rect.width * scale) as u32, (rect.height * scale) as u32);
            if rect.width <= 0 {
                continue;
            }
            let name = out.name();
            let configured = self.config.wallpaper.position_for(&name);
            let key = (w.paths.of(&name)).map(|p| (p.to_path_buf(), placing(p, configured)));
            let position = placed(w.paths.of(&name), configured);
            let s = Shown {
                out,
                rect,
                scale,
                size,
                picture: None,
                position,
            };
            shown.push((s, key));
        }
        w.pictures.retain(|p| {
            (shown.iter()).any(|(_, k)| k.as_ref().is_some_and(|(path, at)| p.is(path, *at)))
        });
        let shown: Vec<Shown> = (shown.into_iter())
            .map(|(mut s, key)| {
                s.picture = key.map(|(path, at)| {
                    (w.pictures.iter().position(|p| p.is(&path, at))).unwrap_or_else(|| {
                        w.pictures.push(Picture::new(&path, at));
                        w.pictures.len() - 1
                    })
                });
                s
            })
            .collect();
        let fading = !w.fading.is_empty();
        let mut loads = Vec::new();
        let mut draws = Vec::new();
        let ready: Vec<bool> = (w.pictures.iter_mut().enumerate())
            .map(|(i, p)| {
                let mine: Vec<&Shown> = shown.iter().filter(|s| s.picture == Some(i)).collect();
                let sizes: Vec<(u32, u32)> = mine.iter().map(|s| s.size).collect();
                p.staged.retain(|s| sizes.contains(&staged_size(s)));
                if p.failed {
                    return false;
                }
                match p.builtin.as_mut() {
                    // Each output shows its own size's picture once it is drawn;
                    // one faded to waits until every output's is.
                    Some((b, drawn)) => {
                        let new = drawn.want(&sizes);
                        if !new.is_empty() {
                            draws.push((p.path.clone(), *b, new));
                        }
                        !fading || sizes.iter().all(|&s| drawn.get(s).is_some())
                    }
                    None => {
                        let position = p.position.unwrap_or_default();
                        let mut want = Cut {
                            position,
                            w: 0,
                            h: 0,
                        };
                        for s in &mine {
                            let scale = if position.scales_to_screen() {
                                s.scale
                            } else {
                                1
                            };
                            want.w = want.w.max((s.rect.width * scale) as u32);
                            want.h = want.h.max((s.rect.height * scale) as u32);
                        }
                        let covered = p.image.as_ref().is_some_and(|l| Cut::covers(l.cut, want));
                        if !covered && want.w > 0 && p.loading != Some(want) {
                            p.loading = Some(want);
                            loads.push((p.path.clone(), p.position, want));
                        }
                        covered
                    }
                }
            })
            .collect();
        let mut changed = false;
        let mut unready = false;
        let mut compose = Vec::new();
        for s in shown {
            let Shown {
                out,
                rect: b,
                scale,
                size,
                picture,
                position,
            } = s;
            let at = w.nodes.iter().position(|(o, _)| *o == out);
            let Some(p) = picture
                .map(|i| (i, &w.pictures[i]))
                .filter(|(_, p)| !p.failed)
            else {
                // The desktop colour alone: no picture here, or one that did not decode.
                if let Some(at) = at {
                    let (_, node) = w.nodes.remove(at);
                    self.scene.destroy(node.buffer);
                    changed = true;
                }
                continue;
            };
            let (i, p) = p;
            let (source, opaque) = match &p.builtin {
                _ if !ready[i] => (None, None),
                // Every built-in is opaque (`wallpapers` pins it): no scan.
                Some((_, drawn)) => (drawn.get(size).map(|p| &**p), Some(true)),
                None => (p.image.as_ref().map(|l| &l.image), None),
            };
            let Some(source) = source else {
                unready = true;
                // Until its own picture is ready an output shows the one before, stretched.
                if let Some(at) = at {
                    changed |= w.nodes[at].1.place(&mut self.scene, b);
                }
                continue;
            };
            let fit = Fit {
                rect: b,
                scale,
                position,
                bg,
            };
            let i_node = match at {
                Some(at) => at,
                None => {
                    let n = self.scene.buffer_create(self.layers.background);
                    // Desktop water draws the picture in its place.
                    self.scene.set_enabled(n, !self.water_on());
                    // Under the desktop icons, which share the layer, and
                    // over the picture it fades in from.
                    self.scene.lower_to_bottom(n);
                    for &old in &w.fading {
                        self.scene.lower_to_bottom(old);
                    }
                    if fading {
                        self.scene.set_opacity(n, 0.0);
                    }
                    w.nodes.push((
                        out,
                        WallpaperNode {
                            buffer: n,
                            fit: None,
                            placed: None,
                            composing: None,
                            texture: None,
                            fades: fading,
                        },
                    ));
                    w.nodes.len() - 1
                }
            };
            let node = &mut w.nodes[i_node].1;
            if node.fit == Some(fit) {
                node.composing = None;
                changed |= node.place(&mut self.scene, b);
                continue;
            }
            let fading_in = node.fit.is_none() && node.fades;
            // A file's picture is composed and staged on a job, so the desktop water samples
            // the texture it shows.
            if let Some(image) = p.image.as_ref() {
                changed |= node.place(&mut self.scene, b);
                if node.composing != Some(fit) {
                    node.composing = Some(fit);
                    compose.push((i_node, fit, size, Arc::clone(image), fading_in));
                }
                continue;
            }
            self.scene.set_position(node.buffer, b.x, b.y);
            let canvas = crate::wallpaper::composed_known(
                source,
                size.0,
                size.1,
                position,
                bg,
                scale as f32,
                opaque,
            );
            let texture = match staged_of(&p.staged, size) {
                // Borrowed: the picture is shown as drawn, so what was staged is the canvas.
                Some(st) if matches!(canvas, Cow::Borrowed(_)) => {
                    let renderer = self.backend.as_mut().map(|b| &mut b.renderer);
                    st.show(&mut self.scene, renderer, node.buffer, &canvas, scale)
                }
                _ => {
                    upload_scaled(&mut self.scene, node.buffer, &canvas, scale);
                    None
                }
            };
            node.shown(fit, texture.clone());
            changed = true;
            let out = w.nodes[i_node].0.clone();
            self.water_wallpaper(&out, Pane::Desktop, &canvas, texture, fading_in);
        }
        w.unready = unready;
        let compose: Vec<_> = (compose.into_iter())
            .map(|(i, fit, size, image, fading_in)| {
                (w.nodes[i].0.clone(), fit, size, image, fading_in)
            })
            .collect();
        let starts_fade = w.fade_due();
        self.wallpaper = Some(w);
        for (path, position, cut) in loads {
            self.load_wallpaper(path, position, cut);
        }
        for (path, b, sizes) in draws {
            self.draw_builtins(path, b, sizes);
        }
        for (out, fit, size, image, fading_in) in compose {
            self.compose_wallpaper(out, fit, size, image, fading_in);
        }
        if starts_fade {
            self.start_fade();
        }
        if changed {
            self.redraw_all();
        }
        self.water_configured();
    }

    /// Compose `image` for `out`'s `fit` on a job; it shows unless the picture or the fit wanted
    /// changed meanwhile. The last output's picture to land starts the fade from the one before
    /// (`fading_in` holds the water's under it until then).
    fn compose_wallpaper(
        &mut self,
        out: Output,
        fit: Fit,
        size: (u32, u32),
        image: Arc<Loaded>,
        fading_in: bool,
    ) {
        let uploads = self.uploads();
        self.spawn_job(move || {
            let (bg, position, scale) = (fit.bg, fit.position, fit.scale as f32);
            let canvas =
                crate::wallpaper::compose(&image.image, size.0, size.1, position, bg, scale);
            let staged = Staged::new(uploads.as_ref(), &canvas);
            Box::new(move |state: &mut State| {
                let Some(w) = state.wallpaper.as_mut() else {
                    return;
                };
                let current = (w.picture_at(&out.name(), fit.position))
                    .and_then(|p| p.image.as_ref())
                    .is_some_and(|l| Arc::ptr_eq(l, &image));
                if !current {
                    return;
                }
                let Some((_, node)) = w.nodes.iter_mut().find(|(o, _)| *o == out) else {
                    return;
                };
                if !node.takes(fit) {
                    return;
                }
                let b = fit.rect;
                state.scene.set_position(node.buffer, b.x, b.y);
                let renderer = state.backend.as_mut().map(|b| &mut b.renderer);
                let texture =
                    staged.show(&mut state.scene, renderer, node.buffer, &canvas, fit.scale);
                node.shown(fit, texture.clone());
                let fade_due = w.fade_due();
                state.water_wallpaper(&out, Pane::Desktop, &canvas, texture, fading_in);
                if fade_due {
                    state.start_fade();
                }
                crate::lockview::repaint_wall(state, &out);
                state.redraw_all();
            })
        });
    }

    fn uploads(&self) -> Option<Uploads> {
        self.backend.as_ref().map(|b| b.uploads.clone())
    }

    /// The staged texture `out`'s wallpaper buffer shows, unless a job is composing another.
    pub fn wallpaper_texture(&self, out: &Output) -> Option<GlesTexture> {
        let w = self.wallpaper.as_ref()?;
        let (_, node) = w.nodes.iter().find(|(o, _)| o == out)?;
        (node.composing.is_none() && node.fit.is_some()).then(|| node.texture.clone())?
    }

    /// Whether `out` shows the desktop colour alone, with no picture to fade from.
    pub(crate) fn wallpaper_shows_nothing(&self, out: &Output) -> bool {
        self.wallpaper.as_ref().is_none_or(|w| {
            let name = out.name();
            w.shows_nothing_on(&name, self.placement(w, &name))
        })
    }

    /// The lock screen's backdrop on `out`: the desktop's texture once it is composed for the
    /// output's current box and scale, else composed here.
    pub fn lock_wallpaper(&self, out: &Output) -> LockWall {
        let Some(w) = self.wallpaper.as_ref() else {
            return LockWall::None;
        };
        let scale = self.draw_scale_at(Some(out));
        let fit = Fit {
            rect: self.layout_box(Some(out)),
            scale,
            position: self.placement(w, &out.name()),
            bg: self.config.theme.colors.background,
        };
        match w.lock_source(out, fit) {
            LockSource::Shown(t) => LockWall::Shown(t),
            LockSource::Buffer(b) => LockWall::Shared(b),
            LockSource::Composing => LockWall::Composing,
            LockSource::Here => self
                .wallpaper_canvas(out, scale)
                .map_or(LockWall::None, LockWall::Composed),
        }
    }

    /// The wallpaper as `out` shows it, at `scale`. None without a wallpaper or before its
    /// decode lands.
    pub fn wallpaper_canvas(&self, out: &Output, scale: i32) -> Option<Canvas> {
        let b = self.layout_box(Some(out));
        let size = ((b.width * scale) as u32, (b.height * scale) as u32);
        let w = self.wallpaper.as_ref()?;
        let name = out.name();
        let position = self.placement(w, &name);
        let source = w.source(&name, position, size)?;
        (b.width > 0).then(|| {
            crate::wallpaper::compose(
                source,
                size.0,
                size.1,
                position,
                self.config.theme.colors.background,
                scale as f32,
            )
        })
    }

    /// The wallpaper as `out` shows it, composed into `w` x `h` pixels:
    /// the Alt+Tab desktop entry's picture, kept until it would differ. The
    /// background colour alone without a wallpaper or before its decode lands.
    pub fn wallpaper_thumbnail(&mut self, out: Option<&Output>, w: i32, h: i32) -> Rc<Canvas> {
        let (key, size) = self.still_key(out, w, h);
        let (w, h) = key.size;
        let bg = key.bg;
        let fill = || {
            let mut c = Canvas::new(w, h);
            c.fill_rect(c.bounds(), bg);
            c
        };
        let Some(wp) = self.wallpaper.as_mut() else {
            return Rc::new(fill());
        };
        let (position, width) = (key.position, key.layout.width);
        let source = wp.still_source(&key, size);
        wp.still(key, |_| match source {
            Some(src) if width > 0 => {
                crate::wallpaper::compose(src.canvas(), w, h, position, bg, w as f32 / width as f32)
            }
            _ => fill(),
        })
    }

    /// Compose on a job the still `wallpaper_thumbnail` would return, so its first call finds it.
    pub fn warm_wallpaper_thumbnail(&mut self, out: Option<&Output>, w: i32, h: i32) {
        let (key, size) = self.still_key(out, w, h);
        let Some(wp) = self.wallpaper.as_ref() else {
            return;
        };
        if key.layout.width <= 0 || wp.still.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        let Some(src) = wp.still_source(&key, size) else {
            return;
        };
        self.spawn_job(move || {
            let (w, h) = key.size;
            let ratio = w as f32 / key.layout.width as f32;
            let still = crate::wallpaper::compose(src.canvas(), w, h, key.position, key.bg, ratio);
            Box::new(move |state: &mut State| {
                if let Some(wp) = state.wallpaper.as_mut() {
                    wp.land_still(key, size, &src, still);
                }
            })
        });
    }

    /// The still key for `out`'s wallpaper at `w` x `h`, and the output's device size.
    fn still_key(&self, out: Option<&Output>, w: i32, h: i32) -> (StillKey, (u32, u32)) {
        let b = self.layout_box(out);
        let scale = self.draw_scale_at(out);
        let position = match (&self.wallpaper, out) {
            (Some(wp), Some(o)) => self.placement(wp, &o.name()),
            (Some(wp), None) => placed(wp.paths.all().next(), self.config.wallpaper.position),
            (None, _) => self.config.wallpaper.position,
        };
        let key = StillKey {
            out: out.cloned(),
            size: (w.max(1) as u32, h.max(1) as u32),
            layout: b,
            scale,
            position,
            bg: self.config.theme.colors.background,
        };
        (key, ((b.width * scale) as u32, (b.height * scale) as u32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_output_of_a_staged_pictures_size_takes_it() {
        let staged = vec![Staged::new(None, &Canvas::new(1920, 1080))];
        for out in ["first", "second"] {
            let p = staged_of(&staged, (1920, 1080));
            assert_eq!(
                p.map(|p| p.size()),
                Some((1920, 1080)),
                "the {out} 1920x1080 output takes the staged picture"
            );
        }
        assert!(staged_of(&staged, (1280, 1024)).is_none());
    }

    #[test]
    fn a_job_for_a_replaced_wallpaper_is_skipped() {
        let jobs = Jobs::default();
        let first = jobs.next();
        assert_eq!(jobs.run(first, || 1), Some(1));
        let second = jobs.next();
        assert_eq!(
            jobs.run(first, || 1),
            None,
            "the first wallpaper was replaced"
        );
        assert_eq!(jobs.run(second, || 2), Some(2));
    }

    #[test]
    fn a_stopped_decode_of_a_picture_set_again_since_does_not_clear_it() {
        let dir =
            std::env::temp_dir().join(format!("reprise-wallpaperview-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.png"), dir.join("b.png"));
        for p in [&a, &b] {
            image::RgbaImage::from_pixel(4, 4, image::Rgba([9, 9, 9, 255]))
                .save(p)
                .unwrap();
        }
        let mut h = crate::harness::Harness::with_config(crate::config::Config::default());
        h.output("A-1", 320, 200);
        h.state.set_wallpaper(Some(&a));
        let w = h.state.wallpaper.as_ref().unwrap();
        let p = &w.pictures[0];
        let (first, at, cut) = (w.serial, p.position, p.loading.expect("a decode for A"));
        h.state.set_wallpaper(Some(&b));
        h.state.set_wallpaper(Some(&a));
        h.state.wallpaper_loaded(first, &a, at, cut, None);
        assert_eq!(
            shown(&h),
            Some(a.as_path()),
            "the first A's decode was stopped"
        );
        h.until("A's own decode lands", |s| {
            (s.wallpaper.as_ref()).is_some_and(|w| w.pictures.iter().any(|p| p.image.is_some()))
        });
        assert_eq!(shown(&h), Some(a.as_path()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Desktops 2 and 3 have their own built-ins, desktop 1 the main one.
    fn three_desktops() -> crate::harness::Harness {
        let mut config = crate::config::Config::parse(
            "[workspaces]\ncount = 3\n[wallpaper]\nimage = \"reprise:tarn/blue\"\n\
             desktops = [\"\", \"reprise:tarn/dusk\", \"reprise:fold/sand\"]\n",
        )
        .unwrap();
        config.workspace_animation.ms = 0;
        let mut h = crate::harness::Harness::with_config(config);
        h.output("A-1", 320, 200);
        h.state.apply_wallpaper_config();
        h
    }

    fn shown(h: &crate::harness::Harness) -> Option<&Path> {
        h.state.wallpaper.as_ref()?.paths.shared.as_deref()
    }

    #[test]
    fn a_desktop_switchs_fade_steps_on_the_outputs_frames_over_its_time() {
        let mut h = three_desktops();
        h.state.switch_desktop(1);
        h.until("the fade to the next picture runs", |s| {
            s.wallpaper.as_ref().is_some_and(|w| w.fade.is_some())
        });
        let start = h.state.wallpaper.as_ref().unwrap().fade.unwrap();
        let shown = |h: &crate::harness::Harness| {
            let w = h.state.wallpaper.as_ref().unwrap();
            h.state.scene.opacity(w.nodes[0].1.buffer)
        };
        crate::animclock::step(&mut h.state, start + DESKTOP_FADE / 2);
        assert_eq!(shown(&h), Some(0.5), "half way, half shown");
        crate::animclock::assert_steps_until(
            &mut h.state,
            crate::animclock::Anim::Wallpaper,
            &["A-1"],
            start + DESKTOP_FADE,
        );
        assert_eq!(shown(&h), Some(1.0), "shown whole at its end");
        let w = h.state.wallpaper.as_ref().unwrap();
        assert!(w.fading.is_empty(), "the picture before is gone");
    }

    #[test]
    fn each_desktop_fades_to_its_own_picture_and_one_without_to_the_main_one() {
        let mut h = three_desktops();
        assert_eq!(shown(&h), Some(Path::new("reprise:tarn/blue")));
        h.state.switch_desktop(1);
        assert_eq!(shown(&h), Some(Path::new("reprise:tarn/dusk")));
        let w = h.state.wallpaper.as_ref().unwrap();
        assert_eq!(
            w.fade_for, DESKTOP_FADE,
            "no switch animation: a short fade"
        );
        h.state.switch_desktop(0);
        assert_eq!(shown(&h), Some(Path::new("reprise:tarn/blue")));
    }

    #[test]
    fn under_reduce_motion_a_desktop_switch_cuts_to_the_next_picture() {
        let config = crate::config::Config::parse(
            "[effects]\nreduce_motion = true\n[workspaces]\ncount = 3\nanimation = \"slide\"\n\
             [wallpaper]\nimage = \"reprise:tarn/blue\"\n\
             desktops = [\"\", \"reprise:tarn/dusk\"]\n",
        )
        .unwrap();
        let mut h = crate::harness::Harness::with_config(config);
        h.output("A-1", 320, 200);
        h.state.apply_wallpaper_config();
        h.state.switch_desktop(1);
        assert_eq!(shown(&h), Some(Path::new("reprise:tarn/dusk")));
        let w = h.state.wallpaper.as_ref().unwrap();
        assert_eq!(w.fade_for, Duration::ZERO, "no crossfade");
    }

    #[test]
    fn switching_through_desktops_keeps_two_pictures_at_most() {
        let mut h = three_desktops();
        for _ in 0..200 {
            if h.state.wallpaper_ready(&h.state.scene.outputs()[0].clone()) {
                break;
            }
            h.run_for(10);
        }
        for n in [1, 2, 0, 2, 1] {
            h.state.switch_desktop(n);
            let w = h.state.wallpaper.as_ref().unwrap();
            assert!(
                w.fading.len() <= 1,
                "one output: the picture fading out is one buffer, got {}",
                w.fading.len()
            );
        }
        assert_eq!(shown(&h), Some(Path::new("reprise:tarn/dusk")));
        for _ in 0..200 {
            let drawn = |w: &Wallpaper| w.pictures.iter().any(|p| p.shows_something());
            if h.state
                .wallpaper
                .as_ref()
                .is_some_and(|w| w.fading.is_empty() && drawn(w))
            {
                break;
            }
            h.run_for(10);
        }
        let w = h.state.wallpaper.as_ref().unwrap();
        assert!(
            w.fading.is_empty(),
            "the fade ends and drops the picture before"
        );
        assert!(
            w.pictures.iter().any(|p| p.shows_something()),
            "the last desktop's picture is drawn though the ones before were skipped"
        );
    }

    fn settle(h: &mut crate::harness::Harness) {
        for _ in 0..200 {
            if h.state.wallpaper_ready(&h.state.scene.outputs()[0].clone()) {
                break;
            }
            h.run_for(10);
        }
    }

    fn shown_buffer(h: &crate::harness::Harness) -> BufferId {
        h.state.wallpaper.as_ref().unwrap().nodes[0].1.buffer
    }

    #[test]
    fn a_second_switch_before_the_next_picture_is_ready_keeps_the_one_on_screen() {
        let mut h = three_desktops();
        settle(&mut h);
        let first = shown_buffer(&h);
        h.state.switch_desktop(1);
        h.state.switch_desktop(2);
        let w = h.state.wallpaper.as_ref().unwrap();
        assert_eq!(w.fading, vec![first], "desktop 1's picture stays under");
        assert!(h.state.scene.enabled(first));
        assert_eq!(h.state.scene.opacity(first), Some(1.0));
    }

    #[test]
    fn a_switch_mid_fade_fades_from_the_incoming_picture_at_full_opacity() {
        let mut h = three_desktops();
        settle(&mut h);
        h.state.switch_desktop(1);
        for _ in 0..200 {
            if h.state.wallpaper.as_ref().unwrap().fade.is_some() {
                break;
            }
            h.run_for(5);
        }
        assert!(h.state.wallpaper.as_ref().unwrap().fade.is_some());
        let second = shown_buffer(&h);
        h.state.switch_desktop(2);
        let w = h.state.wallpaper.as_ref().unwrap();
        assert_eq!(w.fading, vec![second], "desktop 2's picture stays under");
        assert_eq!(h.state.scene.opacity(second), Some(1.0));
    }

    #[test]
    fn a_built_in_refit_at_a_new_background_shows_its_staged_picture_again() {
        let mut h = crate::harness::Harness::new();
        h.output("A-1", 320, 200);
        h.state.set_wallpaper(Some(Path::new("reprise:tarn/blue")));
        let staged = |h: &crate::harness::Harness| {
            let w = h.state.wallpaper.as_ref().expect("a wallpaper");
            (
                (w.pictures.iter())
                    .flat_map(|p| p.staged.iter().map(staged_size))
                    .collect::<Vec<_>>(),
                w.nodes.len(),
            )
        };
        for _ in 0..200 {
            if staged(&h).1 == 1 {
                break;
            }
            h.run_for(10);
        }
        assert_eq!(
            staged(&h),
            (vec![(320, 200)], 1),
            "the landed picture shows and stays staged"
        );
        h.state.config.theme.colors.background = Color::rgb(32, 64, 96);
        h.state.refit_wallpaper();
        let w = h.state.wallpaper.as_ref().expect("a wallpaper");
        let node = &w.nodes[0].1;
        assert!(
            node.fit.is_some_and(|f| f.bg == Color::rgb(32, 64, 96)),
            "the refit shows the new background's fit"
        );
        assert_eq!(
            staged(&h).0,
            vec![(320, 200)],
            "the staged picture is kept for the next refit"
        );
    }

    #[test]
    fn a_fit_shown_at_once_turns_away_the_job_composing_an_older_one() {
        let mut scene = crate::compositor::DeskScene::new();
        let root = scene.root();
        let fit = |width| Fit {
            rect: Rect::new(0, 0, width, 540),
            scale: 2,
            position: Position::Stretch,
            bg: Color::rgb(0, 0, 0),
        };
        let mut node = WallpaperNode {
            buffer: scene.buffer_create(root),
            fit: Some(fit(1920)),
            placed: Some(fit(1920).rect),
            composing: Some(fit(1280)),
            texture: None,
            fades: false,
        };
        assert!(node.takes(fit(1280)));
        node.shown(fit(960), None);
        assert!(
            !node.takes(fit(1280)),
            "the job for the older fit lands stale"
        );
        assert!(node.fit == Some(fit(960)));
    }

    #[test]
    fn the_desktop_still_is_composed_again_only_for_a_new_key_or_a_landed_picture() {
        let mut w = one("/srv/w/a.jpg");
        let composed = std::cell::Cell::new(0);
        let still = |w: &mut Wallpaper, width: i32| {
            let key = StillKey {
                out: None,
                size: (160, 90),
                layout: Rect::new(0, 0, width, 1080),
                scale: 1,
                position: Position::Stretch,
                bg: Color::rgb(0, 0, 0),
            };
            w.still(key, |_| {
                composed.set(composed.get() + 1);
                Canvas::new(160, 90)
            })
        };
        let first = still(&mut w, 1920);
        assert!(
            Rc::ptr_eq(&first, &still(&mut w, 1920)),
            "kept for the same key"
        );
        assert_eq!(composed.get(), 1);
        still(&mut w, 2560);
        assert_eq!(composed.get(), 2, "a new layout composes again");
        land(&mut w, Canvas::new(4, 4));
        still(&mut w, 2560);
        assert_eq!(composed.get(), 3, "a landed picture composes again");
    }

    #[test]
    fn a_still_composed_on_a_job_is_kept_unless_a_picture_landed_meanwhile() {
        let key = || StillKey {
            out: None,
            size: (160, 90),
            layout: Rect::new(0, 0, 1920, 1080),
            scale: 1,
            position: Position::Stretch,
            bg: Color::rgb(0, 0, 0),
        };
        let size = (1920, 1080);
        let picture = |n| Loaded {
            image: Canvas::new(n, n),
            cut: None,
        };
        let mut w = one("/srv/w/a.jpg");
        assert!(w.still_source(&key(), size).is_none(), "nothing to compose");
        land(&mut w, picture(4).image);
        let src = w.still_source(&key(), size).expect("the file's picture");
        land(&mut w, picture(8).image);
        w.land_still(key(), size, &src, Canvas::new(160, 90));
        assert!(w.still.is_none(), "composed from a picture since replaced");
        let src = w.still_source(&key(), size).expect("the file's picture");
        w.land_still(key(), size, &src, Canvas::new(160, 90));
        w.still(key(), |_| panic!("the landed still is kept for its key"));
    }

    #[test]
    fn the_fade_from_the_picture_before_waits_for_every_output_composing() {
        let mut scene = crate::compositor::DeskScene::new();
        let root = scene.root();
        let fit = Fit {
            rect: Rect::new(0, 0, 1920, 1080),
            scale: 1,
            position: Position::Stretch,
            bg: Color::rgb(0, 0, 0),
        };
        let node = |scene: &mut crate::compositor::DeskScene| WallpaperNode {
            buffer: scene.buffer_create(root),
            fit: None,
            placed: None,
            composing: Some(fit),
            texture: None,
            fades: false,
        };
        let mut w = one("/srv/w/a.jpg");
        w.nodes = vec![
            (output("A-1"), node(&mut scene)),
            (output("B-1"), node(&mut scene)),
        ];
        assert!(!w.fade_due(), "no picture before to fade from");
        w.fading.push(scene.buffer_create(root));
        assert!(!w.fade_due(), "both outputs still composing");
        w.nodes[0].1.shown(fit, None);
        assert!(!w.fade_due(), "the second output still composing");
        w.nodes[1].1.shown(fit, None);
        assert!(w.fade_due(), "every output shows its picture");
    }

    #[test]
    fn the_lock_screen_waits_for_the_file_picture_composing_for_its_output() {
        let mut scene = crate::compositor::DeskScene::new();
        let root = scene.root();
        let fit = |width| Fit {
            rect: Rect::new(0, 0, width, 1080),
            scale: 1,
            position: Position::Stretch,
            bg: Color::rgb(0, 0, 0),
        };
        let out = output("A-1");
        let mut w = one("/srv/w/a.jpg");
        assert!(
            matches!(w.lock_source(&out, fit(1920)), LockSource::Here),
            "before the decode lands the lock screen has nothing to wait for"
        );
        land(&mut w, Canvas::new(4, 4));
        assert!(
            matches!(w.lock_source(&out, fit(1920)), LockSource::Composing),
            "an output the refit has not reached waits for its job"
        );
        w.nodes = vec![(
            out.clone(),
            WallpaperNode {
                buffer: scene.buffer_create(root),
                fit: Some(fit(1280)),
                placed: Some(fit(1280).rect),
                composing: Some(fit(1920)),
                texture: None,
                fades: false,
            },
        )];
        assert!(
            matches!(w.lock_source(&out, fit(1920)), LockSource::Composing),
            "an output showing an older fit waits for its job"
        );
        w.nodes[0].1.shown(fit(1920), None);
        let desktop = w.nodes[0].1.buffer;
        assert!(
            matches!(w.lock_source(&out, fit(1920)), LockSource::Buffer(b) if b == desktop),
            "a picture staged in pixel memory shares the desktop's buffer"
        );
    }

    #[test]
    fn the_lock_screen_shares_the_buffer_of_a_built_in_shown_at_its_fit() {
        let mut scene = crate::compositor::DeskScene::new();
        let root = scene.root();
        let fit = |width| Fit {
            rect: Rect::new(0, 0, width, 1080),
            scale: 1,
            position: Position::Stretch,
            bg: Color::rgb(0, 0, 0),
        };
        let out = output("A-1");
        let mut w = one("reprise:tarn/blue");
        assert!(matches!(w.lock_source(&out, fit(1920)), LockSource::Here));
        let desktop = scene.buffer_create(root);
        w.nodes = vec![(
            out.clone(),
            WallpaperNode {
                buffer: desktop,
                fit: Some(fit(1280)),
                placed: Some(fit(1280).rect),
                composing: None,
                texture: None,
                fades: false,
            },
        )];
        assert!(
            matches!(w.lock_source(&out, fit(1920)), LockSource::Here),
            "the desktop shows another fit"
        );
        w.nodes[0].1.shown(fit(1920), None);
        assert!(
            matches!(w.lock_source(&out, fit(1920)), LockSource::Buffer(b) if b == desktop),
            "a built-in with no staged texture shares the desktop's buffer"
        );
    }

    /// A-1 and C-1 show the main built-in on every desktop, B-1 its own; desktop 2 has its own.
    fn own_picture_on_b() -> crate::harness::Harness {
        let mut config = crate::config::Config::parse(
            "[workspaces]\ncount = 2\n[wallpaper]\nimage = \"reprise:tarn/blue\"\n\
             desktops = [\"\", \"reprise:tarn/dusk\"]\n\
             [[outputs]]\nname = \"B-1\"\nwallpaper = \"reprise:fold/sand\"\n",
        )
        .unwrap();
        config.workspace_animation.ms = 0;
        let mut h = crate::harness::Harness::with_config(config);
        h.state.apply_wallpaper_config();
        h.output("A-1", 320, 200);
        h.output("B-1", 320, 200);
        h.output("C-1", 320, 200);
        h
    }

    fn out(h: &crate::harness::Harness, name: &str) -> Output {
        (h.state.scene.outputs().iter())
            .find(|o| o.name() == name)
            .cloned()
            .unwrap()
    }

    fn canvas(h: &crate::harness::Harness, name: &str) -> Vec<u32> {
        let o = out(h, name);
        h.state.wallpaper_canvas(&o, 1).expect("a picture").pixels
    }

    fn node_of(h: &crate::harness::Harness, name: &str) -> Option<BufferId> {
        let w = h.state.wallpaper.as_ref()?;
        (w.nodes.iter())
            .find(|(o, _)| o.name() == name)
            .map(|(_, n)| n.buffer)
    }

    #[test]
    fn an_output_with_its_own_picture_shows_it_and_the_rest_share_the_main_one() {
        let h = own_picture_on_b();
        let w = h.state.wallpaper.as_ref().unwrap();
        assert_eq!(w.paths.of("B-1"), Some(Path::new("reprise:fold/sand")));
        assert_eq!(w.paths.of("A-1"), Some(Path::new("reprise:tarn/blue")));
        assert_eq!(
            w.pictures.len(),
            2,
            "one picture for A-1 and C-1, one for B-1"
        );
        assert_eq!(canvas(&h, "A-1"), canvas(&h, "C-1"));
        assert_ne!(canvas(&h, "A-1"), canvas(&h, "B-1"));
    }

    #[test]
    fn outputs_placing_one_file_alike_share_its_decode() {
        let dir = std::env::temp_dir().join(format!("reprise-own-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([40, 80, 120, 255]))
            .save(&a)
            .unwrap();
        let text = format!(
            "[wallpaper]\nimage = \"{0}\"\n[[outputs]]\nname = \"C-1\"\nwallpaper_position = \"center\"\n",
            a.display()
        );
        let mut h =
            crate::harness::Harness::with_config(crate::config::Config::parse(&text).unwrap());
        h.state.apply_wallpaper_config();
        h.output("A-1", 320, 200);
        h.output("B-1", 640, 400);
        h.output("C-1", 320, 200);
        let w = h.state.wallpaper.as_ref().unwrap();
        let mut held: Vec<_> = w
            .pictures
            .iter()
            .map(|p| (p.position, p.image.is_some()))
            .collect();
        held.sort_by_key(|(p, _)| format!("{p:?}"));
        assert_eq!(
            held,
            [(Some(Position::Center), true), (Some(Position::Fill), true)],
            "A-1 and B-1 fill from one decode, C-1 centres from its own"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_desktop_switch_leaves_an_output_showing_its_own_picture_as_it_is() {
        let mut h = own_picture_on_b();
        let (a, b) = (node_of(&h, "A-1").unwrap(), node_of(&h, "B-1").unwrap());
        h.state.switch_desktop(1);
        let w = h.state.wallpaper.as_ref().unwrap();
        assert_eq!(w.paths.of("A-1"), Some(Path::new("reprise:tarn/dusk")));
        assert_eq!(node_of(&h, "B-1"), Some(b), "B-1 keeps its buffer");
        assert_eq!(h.state.scene.opacity(b), Some(1.0));
        assert!(w.fading.contains(&a), "A-1's picture fades out");
        assert!(!w.fading.contains(&b));
        for _ in 0..200 {
            if h.state.wallpaper.as_ref().unwrap().fading.is_empty() {
                break;
            }
            h.run_for(10);
            crate::animclock::step(&mut h.state, Instant::now() + DESKTOP_FADE);
        }
        assert!(h.state.wallpaper.as_ref().unwrap().fading.is_empty());
        assert_eq!(node_of(&h, "B-1"), Some(b));
        assert_eq!(canvas(&h, "A-1"), canvas(&h, "C-1"));
        assert_ne!(canvas(&h, "A-1"), canvas(&h, "B-1"));
    }

    #[test]
    fn an_output_plugged_in_again_shows_its_own_picture() {
        let mut h = own_picture_on_b();
        let own = canvas(&h, "B-1");
        let b = out(&h, "B-1");
        h.state.scene.remove_output(&b);
        h.state.output_removed(&b);
        h.dispatch();
        assert_eq!(node_of(&h, "B-1"), None);
        h.output("B-1", 320, 200);
        assert_eq!(canvas(&h, "B-1"), own);
    }

    #[test]
    fn an_own_picture_that_does_not_decode_leaves_its_output_the_colour_alone() {
        let dir = std::env::temp_dir().join(format!("reprise-own-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("bad.png");
        std::fs::write(&bad, b"not a picture").unwrap();
        let text = format!(
            "[wallpaper]\nimage = \"reprise:tarn/blue\"\n[[outputs]]\nname = \"B-1\"\nwallpaper = \"{}\"\n",
            bad.display()
        );
        let mut h =
            crate::harness::Harness::with_config(crate::config::Config::parse(&text).unwrap());
        h.state.apply_wallpaper_config();
        h.output("A-1", 320, 200);
        h.output("B-1", 320, 200);
        assert!(node_of(&h, "A-1").is_some(), "A-1 shows the main picture");
        assert_eq!(node_of(&h, "B-1"), None);
        assert!(h.state.wallpaper_shows_nothing(&out(&h, "B-1")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `path` on every output, stretched, with its picture made.
    fn one(path: &str) -> Wallpaper {
        let path = Path::new(path);
        let mut w = Wallpaper::new(Paths::one(Some(path)));
        w.pictures
            .push(Picture::new(path, placing(path, Position::Stretch)));
        w
    }

    fn land(w: &mut Wallpaper, image: Canvas) {
        let p = &w.pictures[0];
        let (path, at) = (p.path.clone(), p.position);
        w.land_image(&path, at, Loaded { image, cut: None });
    }

    fn output(name: &str) -> Output {
        let props = smithay::output::PhysicalProperties {
            size: (0, 0).into(),
            subpixel: smithay::output::Subpixel::Unknown,
            make: "Reprise".into(),
            model: "Test".into(),
            serial_number: "0".into(),
        };
        Output::new(name.into(), props)
    }
}
