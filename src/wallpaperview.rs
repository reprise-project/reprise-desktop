//! The desktop wallpaper (`[wallpaper]`), decoded (a built-in is drawn at each
//! output's own size) off the loop and composed per output into
//! `layers.background`, and the slideshow: every `interval` minutes a thread
//! picks the folder's next picture and the new buffers fade in over the old.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use reprisekit::wallpapers::Builtin;
use reprisescene::BufferId;
use smithay::{
    output::Output,
    reexports::calloop::{
        RegistrationToken, channel,
        timer::{TimeoutAction, Timer},
    },
};

use crate::{
    compositor::State,
    dmaupload::{Staged, Uploads},
    output::upload_scaled,
    raster::{Canvas, Rect},
    theme::Color,
    wallpaper::{Cut, Drawn, Loaded, Position},
    waterview::Pane,
};

/// The wallpaper and its scene buffers, one per output.
pub struct Wallpaper {
    pub path: PathBuf,
    /// A file's picture; None until the first decode lands, and for a
    /// built-in. Shared with the jobs composing it.
    image: Option<Arc<Loaded>>,
    /// The cut a decode in flight is making; a finished decode for any
    /// other is stale and dropped.
    loading: Option<Cut>,
    /// A built-in and its pictures, one per output device size.
    builtin: Option<(Builtin, Drawn)>,
    /// Built-in pictures made ready for the scene on the drawing thread; the first output of
    /// a picture's size takes it.
    staged: Vec<Staged>,
    nodes: Vec<(Output, WallpaperNode)>,
    /// The picture this one replaces, still on screen under it until this
    /// one has faded in: its buffers, and the fade's timer and start once
    /// it runs.
    fading: Vec<BufferId>,
    fade: Option<(RegistrationToken, Instant)>,
    /// The Alt+Tab desktop entry's last picture and what it was composed for; a picture
    /// landing drops it.
    still: Option<(StillKey, Rc<Canvas>)>,
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
    pub fn new(path: &Path) -> Wallpaper {
        Wallpaper {
            path: path.to_path_buf(),
            image: None,
            loading: None,
            builtin: crate::wallpaper::builtin(path).map(|b| (b, Drawn::default())),
            staged: Vec::new(),
            nodes: Vec::new(),
            fading: Vec::new(),
            fade: None,
            still: None,
        }
    }

    fn land_image(&mut self, loaded: Loaded) {
        self.image = Some(Arc::new(loaded));
        self.still = None;
    }

    /// A built-in's picture lands; false (and dropped) when no output wants its size any more.
    fn land_drawn(&mut self, picture: Canvas) -> bool {
        let landed = self.builtin.as_mut().is_some_and(|(_, d)| d.land(picture));
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

    pub(crate) fn shows_nothing(&self) -> bool {
        self.image.is_none()
            && self.builtin.as_ref().is_none_or(|(_, d)| d.any().is_none())
            && self.fading.is_empty()
    }

    /// The picture an output of `size` device pixels is composed from:
    /// a built-in's own picture of that size, else the file's.
    fn source(&self, size: (u32, u32)) -> Option<&Canvas> {
        match &self.builtin {
            Some((_, d)) => d.get(size),
            None => self.image.as_ref().map(|l| &l.image),
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

/// How long a slideshow picture takes to fade into the next, and the
/// step of that fade.
const FADE: Duration = Duration::from_millis(1200);
const FADE_TICK: Duration = Duration::from_millis(33);

/// The running `[wallpaper] slideshow`: its folder, the timer for the
/// next change, and whether a pick is on its way from the thread.
pub struct Slideshow {
    folder: PathBuf,
    timer: Option<RegistrationToken>,
    picking: bool,
}

struct WallpaperNode {
    buffer: BufferId,
    fit: Option<Fit>,
    /// The logical box the buffer covers; None until it first holds a picture.
    placed: Option<Rect>,
    /// The fit a job is composing for this output.
    composing: Option<Fit>,
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
}

/// Whether a file's picture of `img` pixels, shown on an output that already shows a picture, is
/// composed for `size` on a job: anything but a stretched picture of the output's own size
/// resamples or fills a whole screen.
fn composes_off_loop(shown: bool, position: Position, img: (u32, u32), size: (u32, u32)) -> bool {
    shown && !(position == Position::Stretch && img == size)
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

impl State {
    /// Put `path` (or nothing) under the desktop icons on every output.
    pub fn set_wallpaper(&mut self, path: Option<&Path>) {
        let same = match (&self.wallpaper, path) {
            (Some(w), Some(p)) => w.path == p,
            (None, None) => true,
            _ => false,
        };
        // A built-in fades in over the picture before it (a new glass
        // colour retints Glass's), which stays until it is drawn.
        if let Some(p) = path
            && !same
            && self.wallpaper.is_some()
            && crate::wallpaper::builtin(p).is_some()
        {
            self.fade_wallpaper_to(p);
            return;
        }
        if !same {
            self.clear_wallpaper();
            self.wallpaper = path.map(Wallpaper::new);
        }
        self.refit_wallpaper();
    }

    /// Decode the wallpaper cut to `cut` on a thread. On the loop it
    /// replaces the image and recomposes every output, unless the
    /// wallpaper or the cut wanted changed meanwhile; a file that fails to
    /// decode leaves no wallpaper.
    fn load_wallpaper(&mut self, path: PathBuf, cut: Cut) {
        let (tx, rx) = channel::channel::<Option<Loaded>>();
        let inserted = self.handle.insert_source(rx, move |event, _, state| {
            let channel::Event::Msg(loaded) = event else {
                return;
            };
            let Some(w) = state.wallpaper.as_mut() else {
                return;
            };
            if w.path != path || w.loading != Some(cut) {
                return;
            }
            w.loading = None;
            match loaded {
                Some(l) => {
                    w.land_image(l);
                    for (_, node) in &mut w.nodes {
                        node.fit = None;
                        node.composing = None;
                    }
                    state.refit_wallpaper();
                    crate::lockview::refit(state);
                }
                None => state.clear_wallpaper(),
            }
        });
        if let Err(e) = inserted {
            log::warn!("wallpaper: {}", e.error);
            return;
        }
        let file = self.wallpaper.as_ref().map(|w| w.path.clone());
        let spawned = std::thread::Builder::new()
            .name("wallpaper".into())
            .spawn(move || {
                let loaded = file.and_then(|f| crate::wallpaper::load(&f, Some(cut)));
                let _ = tx.send(loaded);
            });
        if let Err(e) = spawned {
            log::warn!("wallpaper: no decode thread: {e}");
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
            if w.path != path {
                return;
            }
            if w.land_drawn(picture) {
                w.staged.push(staged);
                state.refit_wallpaper();
                crate::lockview::refit(state);
            }
        });
        if let Err(e) = inserted {
            log::warn!("wallpaper: {}", e.error);
            return;
        }
        let uploads = self.uploads();
        let spawned = std::thread::Builder::new()
            .name("wallpaper".into())
            .spawn(move || {
                for (w, h) in sizes {
                    let picture = crate::wallpaper::draw(b, w, h);
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
            self.end_fade(w.fading, w.fade);
            self.water_configured();
            crate::lockview::refit(self);
            self.redraw_all();
        }
    }

    fn end_fade(&mut self, fading: Vec<BufferId>, fade: Option<(RegistrationToken, Instant)>) {
        for b in fading {
            self.scene.destroy(b);
        }
        if let Some((t, _)) = fade {
            self.handle.remove(t);
        }
    }

    /// Fade from the wallpaper showing to `path`: the old buffers stay
    /// until the new picture is decoded and has faded in over them. With
    /// nothing showing, `path` simply shows.
    pub fn fade_wallpaper_to(&mut self, path: &Path) {
        if self.wallpaper.as_ref().is_some_and(|w| w.path == path) {
            return;
        }
        let mut next = Wallpaper::new(path);
        if let Some(old) = self.wallpaper.take() {
            self.end_fade(old.fading, old.fade);
            next.fading = old.nodes.into_iter().map(|(_, n)| n.buffer).collect();
        }
        self.wallpaper = Some(next);
        self.refit_wallpaper();
    }

    fn start_fade(&mut self) {
        let timer = self
            .handle
            .insert_source(Timer::from_duration(FADE_TICK), |_, _, state| {
                let Some(w) = state.wallpaper.as_mut() else {
                    return TimeoutAction::Drop;
                };
                let t = w.fade.map_or(1.0, |(_, at)| {
                    at.elapsed().as_secs_f32() / FADE.as_secs_f32()
                });
                let t = t.min(1.0);
                for (_, n) in &w.nodes {
                    state.scene.set_opacity(n.buffer, t);
                }
                let fading = if t < 1.0 {
                    Vec::new()
                } else {
                    w.fade = None;
                    std::mem::take(&mut w.fading)
                };
                let done = t >= 1.0;
                state.water_fade(t);
                for b in fading {
                    state.scene.destroy(b);
                }
                state.redraw_all();
                if done {
                    TimeoutAction::Drop
                } else {
                    TimeoutAction::ToDuration(FADE_TICK)
                }
            });
        match timer {
            Ok(t) => {
                if let Some(w) = self.wallpaper.as_mut() {
                    w.fade = Some((t, Instant::now()));
                }
            }
            Err(e) => log::warn!("wallpaper fade: {}", e.error),
        }
    }

    /// Follow `[wallpaper]`'s image and slideshow keys after a reload, and
    /// at startup: with a slideshow its folder's pictures take turns (a
    /// new folder picks at once), else `image` shows.
    pub fn apply_wallpaper_config(&mut self) {
        let Some(folder) = self.config.wallpaper.slideshow.clone() else {
            if let Some(t) = self.slideshow.take().and_then(|s| s.timer) {
                self.handle.remove(t);
            }
            let paper = self.config.wallpaper.image.clone();
            self.set_wallpaper(paper.as_deref());
            return;
        };
        if self.slideshow.as_ref().is_some_and(|s| s.folder == folder) {
            // The interval may have changed: count it from now.
            self.arm_slideshow();
            return;
        }
        if let Some(t) = self.slideshow.take().and_then(|s| s.timer) {
            self.handle.remove(t);
        }
        self.slideshow = Some(Slideshow {
            folder,
            timer: None,
            picking: false,
        });
        self.next_slide();
    }

    /// Whether `out` shows its wallpaper, or has none to wait for.
    pub fn wallpaper_ready(&self, out: &Output) -> bool {
        if self.slideshow.as_ref().is_some_and(|s| s.picking) {
            return false;
        }
        self.wallpaper
            .as_ref()
            .is_none_or(|w| w.nodes.iter().any(|(o, n)| o == out && n.fit.is_some()))
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
        let current = self.wallpaper.as_ref().map(|w| w.path.clone());
        let Some(show) = self.slideshow.as_mut() else {
            return;
        };
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
                Some(p) => state.fade_wallpaper_to(&p),
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
            Err(e) => log::warn!("slideshow timer: {}", e.error),
        }
    }

    /// Compose the wallpaper for every output's current size and position; an
    /// output whose fit has not changed keeps its buffer. A built-in is drawn at
    /// each output size not yet drawn, and pictures of sizes no output shows are
    /// dropped.
    pub fn refit_wallpaper(&mut self) {
        let outputs: Vec<Output> = self.scene.outputs().to_vec();
        let bg = self.config.theme.colors.background;
        let position = self.config.wallpaper.position;
        // Taken out for the loop so the image is borrowed, not cloned,
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
        let shown: Vec<(Output, Rect, i32, (u32, u32))> = outputs
            .into_iter()
            .filter_map(|out| {
                let b = self.layout_box(Some(&out));
                let scale = self.draw_scale_at(Some(&out));
                let size = ((b.width * scale) as u32, (b.height * scale) as u32);
                (b.width > 0).then_some((out, b, scale, size))
            })
            .collect();
        let sizes: Vec<(u32, u32)> = shown.iter().map(|s| s.3).collect();
        w.staged
            .retain(|p| sizes.contains(&(p.size().0 as u32, p.size().1 as u32)));
        let mut load = None;
        let mut draw = Vec::new();
        let ready = match w.builtin.as_mut() {
            // Each output shows its own size's picture once it is drawn;
            // one faded to waits until every output's is.
            Some((_, drawn)) => {
                draw = drawn.want(&sizes);
                w.fading.is_empty() || sizes.iter().all(|&s| drawn.get(s).is_some())
            }
            None => {
                let mut want = Cut {
                    position,
                    w: 0,
                    h: 0,
                };
                for (_, b, scale, _) in &shown {
                    let scale = match position {
                        Position::Stretch => *scale,
                        _ => 1,
                    };
                    want.w = want.w.max((b.width * scale) as u32);
                    want.h = want.h.max((b.height * scale) as u32);
                }
                let covered = w.image.as_ref().is_some_and(|l| Cut::covers(l.cut, want));
                if !covered && want.w > 0 && w.loading != Some(want) {
                    w.loading = Some(want);
                    load = Some(want);
                }
                covered
            }
        };
        let mut changed = false;
        let mut fade_in = false;
        let mut compose = Vec::new();
        for (out, b, scale, size) in shown {
            let source = match &w.builtin {
                _ if !ready => None,
                Some((_, drawn)) => drawn.get(size),
                None => w.image.as_ref().map(|l| &l.image),
            };
            let Some(source) = source else {
                // Until its own picture is ready an output shows the one before, stretched.
                if let Some((_, node)) = w.nodes.iter_mut().find(|(o, _)| *o == out) {
                    changed |= node.place(&mut self.scene, b);
                }
                continue;
            };
            let fit = Fit {
                rect: b,
                scale,
                position,
                bg,
            };
            let mut fresh = false;
            let i = match w.nodes.iter().position(|(o, _)| *o == out) {
                Some(i) => i,
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
                    if !w.fading.is_empty() {
                        self.scene.set_opacity(n, 0.0);
                        fade_in = true;
                        fresh = true;
                    }
                    w.nodes.push((
                        out,
                        WallpaperNode {
                            buffer: n,
                            fit: None,
                            placed: None,
                            composing: None,
                        },
                    ));
                    w.nodes.len() - 1
                }
            };
            let node = &mut w.nodes[i].1;
            if node.fit == Some(fit) {
                node.composing = None;
                changed |= node.place(&mut self.scene, b);
                continue;
            }
            let img = (source.width, source.height);
            if let Some(image) = w
                .image
                .as_ref()
                .filter(|_| composes_off_loop(node.placed.is_some(), position, img, size))
            {
                changed |= node.place(&mut self.scene, b);
                if node.composing != Some(fit) {
                    node.composing = Some(fit);
                    compose.push((i, fit, size, Arc::clone(image)));
                }
                continue;
            }
            self.scene.set_position(node.buffer, b.x, b.y);
            let canvas =
                crate::wallpaper::composed(source, size.0, size.1, position, bg, scale as f32);
            let staged = w
                .staged
                .iter()
                .position(|p| p.size() == (size.0 as i32, size.1 as i32))
                .map(|i| w.staged.swap_remove(i));
            match staged {
                // Borrowed: the picture is shown as drawn, so what was staged is the canvas.
                Some(p) if matches!(canvas, Cow::Borrowed(_)) => {
                    let renderer = self.backend.as_mut().map(|b| &mut b.renderer);
                    p.show(&mut self.scene, renderer, node.buffer, &canvas, scale);
                }
                _ => upload_scaled(&mut self.scene, node.buffer, &canvas, scale),
            }
            node.fit = Some(fit);
            node.placed = Some(b);
            changed = true;
            let out = w.nodes[i].0.clone();
            self.water_wallpaper(&out, Pane::Desktop, &canvas, fresh);
        }
        let compose: Vec<_> = (compose.into_iter())
            .map(|(i, fit, size, image)| (w.nodes[i].0.clone(), fit, size, image))
            .collect();
        let starts_fade = fade_in && w.fade.is_none();
        let path = w.path.clone();
        let builtin = w.builtin.as_ref().map(|(b, _)| *b);
        self.wallpaper = Some(w);
        if let Some(cut) = load {
            self.load_wallpaper(path.clone(), cut);
        }
        if let Some(b) = builtin.filter(|_| !draw.is_empty()) {
            self.draw_builtins(path, b, draw);
        }
        for (out, fit, size, image) in compose {
            self.compose_wallpaper(out, fit, size, image);
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
    /// changed meanwhile.
    fn compose_wallpaper(&mut self, out: Output, fit: Fit, size: (u32, u32), image: Arc<Loaded>) {
        let uploads = self.uploads();
        self.spawn_fs_job(move || {
            let (bg, position, scale) = (fit.bg, fit.position, fit.scale as f32);
            let canvas =
                crate::wallpaper::compose(&image.image, size.0, size.1, position, bg, scale);
            let staged = Staged::new(uploads.as_ref(), &canvas);
            Box::new(move |state: &mut State| {
                let Some(w) = state.wallpaper.as_mut() else {
                    return;
                };
                if !w.image.as_ref().is_some_and(|l| Arc::ptr_eq(l, &image)) {
                    return;
                }
                let Some((_, node)) = w.nodes.iter_mut().find(|(o, _)| *o == out) else {
                    return;
                };
                if node.composing != Some(fit) {
                    return;
                }
                node.composing = None;
                node.fit = Some(fit);
                node.placed = Some(fit.rect);
                let b = fit.rect;
                state.scene.set_position(node.buffer, b.x, b.y);
                let renderer = state.backend.as_mut().map(|b| &mut b.renderer);
                staged.show(&mut state.scene, renderer, node.buffer, &canvas, fit.scale);
                state.water_wallpaper(&out, Pane::Desktop, &canvas, false);
                state.redraw_all();
            })
        });
    }

    fn uploads(&self) -> Option<Uploads> {
        self.backend.as_ref().map(|b| b.uploads.clone())
    }

    /// The wallpaper as `out` shows it, at `scale`: the lock screen's
    /// backdrop. None without a wallpaper or before its decode lands.
    pub fn wallpaper_canvas(&self, out: &Output, scale: i32) -> Option<Canvas> {
        let b = self.layout_box(Some(out));
        let size = ((b.width * scale) as u32, (b.height * scale) as u32);
        let source = self.wallpaper.as_ref()?.source(size)?;
        (b.width > 0).then(|| {
            crate::wallpaper::compose(
                source,
                size.0,
                size.1,
                self.config.wallpaper.position,
                self.config.theme.colors.background,
                scale as f32,
            )
        })
    }

    /// The wallpaper as `out` shows it, composed into `w` x `h` pixels:
    /// the Alt+Tab desktop entry's picture, kept until it would differ. The
    /// background colour alone without a wallpaper or before its decode lands.
    pub fn wallpaper_thumbnail(&mut self, out: Option<&Output>, w: i32, h: i32) -> Rc<Canvas> {
        let (w, h) = (w.max(1) as u32, h.max(1) as u32);
        let bg = self.config.theme.colors.background;
        let position = self.config.wallpaper.position;
        let b = self.layout_box(out);
        // A built-in's picture of the output's own size; any of them for
        // the whole layout.
        let scale = self.draw_scale_at(out);
        let size = ((b.width * scale) as u32, (b.height * scale) as u32);
        let fill = || {
            let mut c = Canvas::new(w, h);
            c.fill_rect(c.bounds(), bg);
            c
        };
        let Some(wp) = self.wallpaper.as_mut() else {
            return Rc::new(fill());
        };
        let key = StillKey {
            out: out.cloned(),
            size: (w, h),
            layout: b,
            scale,
            position,
            bg,
        };
        wp.still(key, |wp| {
            let image = match (&wp.builtin, out) {
                (Some((_, drawn)), None) => drawn.any(),
                _ => wp.source(size),
            };
            match image {
                Some(image) if b.width > 0 => {
                    crate::wallpaper::compose(image, w, h, position, bg, w as f32 / b.width as f32)
                }
                _ => fill(),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_still_is_composed_again_only_for_a_new_key_or_a_landed_picture() {
        let mut w = Wallpaper::new(Path::new("/srv/w/a.jpg"));
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
        w.land_image(Loaded {
            image: Canvas::new(4, 4),
            cut: None,
        });
        still(&mut w, 2560);
        assert_eq!(composed.get(), 3, "a landed picture composes again");
    }

    #[test]
    fn a_shown_picture_at_a_new_size_composes_on_a_job_and_a_first_show_at_once() {
        let s = Position::Stretch;
        assert!(composes_off_loop(true, s, (2560, 1440), (3072, 1728)));
        assert!(composes_off_loop(true, s, (3072, 1728), (1920, 1080)));
        assert!(composes_off_loop(
            true,
            Position::Center,
            (1920, 1080),
            (1920, 1080)
        ));
        assert!(
            !composes_off_loop(true, s, (1920, 1080), (1920, 1080)),
            "shown as drawn"
        );
        assert!(
            !composes_off_loop(false, s, (2560, 1440), (1920, 1080)),
            "first show"
        );
    }
}
