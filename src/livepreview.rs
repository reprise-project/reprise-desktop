//! A live picture of one window for the task button panel (`taskpreview`),
//! Alt+Tab and the 3D switcher (`flipview`).
//!
//! Each thumb is a texture of its own (`offscreen::Target`) that the
//! window's `content` subtree is rendered into at thumbnail size, or a
//! cell of a texture shared by thumbs made together ([`State::thumbs_create`]),
//! drawn in one render: the
//! client surface and its popups without the frame, drawn even while the
//! window is covered, minimized or on another desktop (the subtree's own
//! and its ancestors' enabled flags are ignored). A scene buffer in the
//! host's tree shows the texture, which later renders update in place: no
//! pixels pass through the CPU.
//!
//! A whole-window thumb ([`State::whole_thumb_create`]) renders the
//! window's whole tree over its frame rect instead (frame art, client,
//! popups; the frame's glass left out, as `windowstill` does, and the
//! shadow outside the rect) at a pixel size the host picks, and hands the
//! texture to the host's [`TextureSlot`] instead of a scene buffer.
//!
//! The pictures follow the window's commits at `[previews] fps` at most
//! (100 ms by default): a commit marks the window's thumbs (a new focus
//! or title marks its whole-window ones, `thumbs_frame`), and one timer
//! draws every marked thumb a period after the last draw. A render draws
//! only what changed since the thumb's last (the target's damage
//! tracker), so a busy window costs its changed pixels and an idle one
//! nothing. Each draw sends the window's frame callbacks, which keeps a
//! minimized client, or one on another desktop, drawing while shown.
//!
//! Hosts keep a [`ThumbId`] and drop it with [`State::thumb_drop`] before
//! the tree it lives in goes.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use reprisekit::controls::taskpreview::fit;
use reprisescene::{BufferId, TreeId, View};
use smithay::{
    backend::renderer::{
        Renderer,
        element::{Element, Id, Kind, RenderElement, UnderlyingStorage, utils::CropRenderElement},
        gles::{GlesRenderer, GlesTexture, ffi},
        utils::{CommitCounter, DamageSet, OpaqueRegions, with_renderer_surface_state},
    },
    output::Output,
    reexports::calloop::{
        RegistrationToken,
        timer::{TimeoutAction, Timer},
    },
    utils::{Buffer, Physical, Point, Rectangle, Scale, Transform, user_data::UserDataMap},
};

use crate::{
    compositor::State, config::PREVIEW_FPS_MAX, offscreen::Target, raster::Rect, window::WindowId,
};

/// Where a whole-window thumb puts its texture for the host to draw: the
/// same texture, drawn into again by every later render, until a new
/// size makes a new one.
pub type TextureSlot = Rc<RefCell<Option<GlesTexture>>>;

/// Names a live picture for as long as its host keeps it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThumbId(u64);

/// Every live picture, and the timer that draws them.
#[derive(Default)]
pub struct LivePreviews {
    thumbs: Vec<Thumb>,
    sheets: Vec<Sheet>,
    next: u64,
    /// Fires a period after the last draw while a thumb waits for one.
    timer: Option<RegistrationToken>,
    /// When the marked thumbs were last drawn.
    drawn: Option<Instant>,
    /// The widest and tallest a sheet's texture grows, read from the renderer once.
    sheet_cap: Option<i32>,
}

/// One live picture: where it shows and the texture behind.
struct Thumb {
    id: ThumbId,
    window: WindowId,
    show: Show,
    /// The texture, sized to `mode`; None until the first draw or after
    /// one failed.
    target: Option<Target>,
    /// The picture's size in device pixels and the size it maps.
    mode: (i32, i32, i32, i32),
    /// The window committed since the last draw.
    marked: bool,
    /// The shared texture the picture is a cell of, and the cell's page
    /// and top-left there (None until laid out). `target` stays None.
    sheet: Option<u64>,
    cell: Option<(usize, Point<i32, Physical>)>,
}

/// Textures shared by well thumbs made together: one render per page, one
/// GPU submit, draws them all.
struct Sheet {
    id: u64,
    /// Empty until the first draw or after one failed.
    pages: Vec<Target>,
}

/// Device pixels left clear between two cells of a sheet, so a filtered
/// draw of one cell never samples its neighbour.
const SHEET_GAP: i32 = 2;
/// The widest a sheet page grows before its cells start a new row, and the
/// tallest before they start a new page, unless the renderer's
/// `GL_MAX_TEXTURE_SIZE` is smaller.
const SHEET_MAX: i32 = 4096;

/// What a thumb pictures and where the picture goes.
enum Show {
    /// The client, fitted to `well` (logical, relative to the buffer's
    /// parent) at `scale` device pixels per logical pixel, in `buffer`.
    Well {
        buffer: BufferId,
        well: Rect,
        scale: i32,
    },
    /// The whole window, `px` device pixels large, into `slot`, with
    /// mipmaps (the 3D switcher draws it smaller).
    Whole { px: (i32, i32), slot: TextureSlot },
}

/// What a draw needs of the thumb's window: the tree rendered and the
/// layout rect it covers (the client's for `content`, the frame's for the
/// whole window).
struct Shot {
    tree: TreeId,
    rect: Rect,
    surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
}

impl State {
    /// Whether `w` has something to show: a mapped surface with a buffer
    /// and a size.
    pub fn has_picture(&self, w: WindowId) -> bool {
        self.win(w).is_some_and(|x| {
            x.mapped
                && x.client_w > 0
                && x.client_h > 0
                && with_renderer_surface_state(x.wl_surface(), |s| s.buffer().is_some())
                    .unwrap_or(false)
        })
    }

    /// Show `w` live in `well` (logical, relative to `parent`), at `scale`
    /// device pixels per logical pixel, and draw its first picture now.
    /// None when the window has nothing to show.
    pub fn thumb_create(
        &mut self,
        w: WindowId,
        parent: TreeId,
        well: Rect,
        scale: i32,
    ) -> Option<ThumbId> {
        if !self.has_picture(w) {
            return None;
        }
        let buffer = self.scene.buffer_create(parent);
        let show = Show::Well {
            buffer,
            well,
            scale: scale.max(1),
        };
        let id = self.thumb_push(w, show, None);
        self.draw_thumbs(Some(&[id]));
        Some(id)
    }

    /// [`State::thumb_create`] for each (window, well) of `wants`, all
    /// under `parent` at `scale`, drawn together into one shared texture.
    pub fn thumbs_create(
        &mut self,
        parent: TreeId,
        wants: &[(WindowId, Rect)],
        scale: i32,
    ) -> Vec<Option<ThumbId>> {
        let sheet = self.live_previews.next;
        self.live_previews.next += 1;
        self.live_previews.sheets.push(Sheet {
            id: sheet,
            pages: Vec::new(),
        });
        let mut seen = Vec::new();
        let ids: Vec<Option<ThumbId>> = wants
            .iter()
            .map(|&(w, well)| {
                if !self.has_picture(w) {
                    return None;
                }
                let buffer = self.scene.buffer_create(parent);
                let show = Show::Well {
                    buffer,
                    well,
                    scale: scale.max(1),
                };
                // A window's elements appear once per render: a second
                // picture of it gets its own texture.
                let shared = (!seen.contains(&w)).then_some(sheet);
                seen.push(w);
                Some(self.thumb_push(w, show, shared))
            })
            .collect();
        let drawn: Vec<ThumbId> = ids.iter().flatten().copied().collect();
        self.sheet_forget_unused();
        self.draw_thumbs(Some(&drawn));
        ids
    }

    /// Show the whole of `w` live, `px` device pixels large, in `slot`,
    /// and draw its first picture now. None when the window has nothing to
    /// show.
    pub fn whole_thumb_create(
        &mut self,
        w: WindowId,
        px: (i32, i32),
        slot: TextureSlot,
    ) -> Option<ThumbId> {
        if !self.has_picture(w) {
            return None;
        }
        let id = self.thumb_push(w, Show::Whole { px, slot }, None);
        self.draw_thumbs(Some(&[id]));
        Some(id)
    }

    fn thumb_push(&mut self, w: WindowId, show: Show, sheet: Option<u64>) -> ThumbId {
        let p = &mut self.live_previews;
        let id = ThumbId(p.next);
        p.next += 1;
        p.thumbs.push(Thumb {
            id,
            window: w,
            show,
            target: None,
            mode: (0, 0, 0, 0),
            marked: false,
            sheet,
            cell: None,
        });
        id
    }

    /// Drop the sheets no thumb is a cell of.
    fn sheet_forget_unused(&mut self) {
        let p = &mut self.live_previews;
        let thumbs = &p.thumbs;
        p.sheets
            .retain(|s| thumbs.iter().any(|t| t.sheet == Some(s.id)));
    }

    pub fn thumb_drop(&mut self, id: ThumbId) {
        let p = &mut self.live_previews;
        let Some(i) = p.thumbs.iter().position(|t| t.id == id) else {
            return;
        };
        let t = p.thumbs.remove(i);
        if !p.thumbs.iter().any(|t| t.marked)
            && let Some(token) = p.timer.take()
        {
            self.handle.remove(token);
        }
        if let Show::Well { buffer, .. } = t.show {
            self.scene.destroy(buffer);
        }
        if t.sheet.is_some() {
            self.sheet_forget_unused();
        }
    }

    pub fn thumb_shows(&self, id: ThumbId, w: WindowId, well: Rect, scale: i32) -> bool {
        self.live_previews.thumbs.iter().any(|t| {
            t.id == id
                && t.window == w
                && matches!(t.show, Show::Well { well: at, scale: s, .. }
                    if at == well && s == scale.max(1))
        })
    }

    /// The shortest time between two draws of a live picture: one frame
    /// of `[previews] fps`.
    pub fn preview_period(&self) -> Duration {
        let fps = self.config.previews.fps.clamp(1, PREVIEW_FPS_MAX);
        Duration::from_millis(1000 / u64::from(fps))
    }

    /// `w` committed: its pictures draw again within `[previews] fps`.
    pub fn thumbs_commit(&mut self, w: WindowId) {
        self.thumbs_mark(w, false);
    }

    /// `w`'s frame changed (focus, title): its whole-window pictures draw
    /// again within `[previews] fps`.
    pub fn thumbs_frame(&mut self, w: WindowId) {
        self.thumbs_mark(w, true);
    }

    /// Mark `w`'s thumbs, only the whole-window ones with `whole`, and
    /// arm the timer that draws them.
    fn thumbs_mark(&mut self, w: WindowId, whole: bool) {
        let p = &mut self.live_previews;
        let mut any = false;
        for t in p.thumbs.iter_mut().filter(|t| t.window == w) {
            if whole && !matches!(t.show, Show::Whole { .. }) {
                continue;
            }
            t.marked = true;
            any = true;
        }
        if !any || p.timer.is_some() {
            return;
        }
        let wait = preview_wait(p.drawn, Instant::now(), self.preview_period());
        let token = self
            .handle
            .insert_source(Timer::from_duration(wait), |_, _, state| {
                state.live_previews.timer = None;
                state.draw_thumbs(None);
                TimeoutAction::Drop
            });
        match token {
            Ok(t) => self.live_previews.timer = Some(t),
            Err(e) => log::warn!("live preview timer: {}", e.error),
        }
    }

    /// Draw the thumbs `only`, or every marked thumb for None, and every
    /// other cell of their sheets: fit the client's current size, render
    /// what changed and send the window's frame callbacks.
    fn draw_thumbs(&mut self, only: Option<&[ThumbId]>) {
        let picked = |t: &Thumb| only.map_or(t.marked, |ids| ids.contains(&t.id));
        let thumbs = &self.live_previews.thumbs;
        let sheets: Vec<u64> = thumbs
            .iter()
            .filter(|t| picked(t))
            .filter_map(|t| t.sheet)
            .collect();
        let jobs: Vec<(usize, Shot)> = thumbs
            .iter()
            .enumerate()
            .filter(|(_, t)| picked(t) || t.sheet.is_some_and(|s| sheets.contains(&s)))
            .filter_map(|(i, t)| {
                let x = self.win(t.window)?;
                let (tree, rect) = match t.show {
                    Show::Well { .. } => (x.content, Rect::new(0, 0, x.client_w, x.client_h)),
                    Show::Whole { .. } => (x.tree, x.frame_rect()),
                };
                Some((
                    i,
                    Shot {
                        tree,
                        rect,
                        surface: x.wl_surface().clone(),
                    },
                ))
            })
            .collect();
        if only.is_none() {
            self.live_previews.drawn = Some(Instant::now());
        }
        let now = self.now();
        let output = self.scene.outputs().first().cloned();
        let shown_on = self.thumb_outputs(jobs.iter().map(|(i, _)| *i));
        let State {
            backend,
            scene,
            live_previews,
            glass,
            config,
            ..
        } = self;
        let Some(renderer) = backend.as_mut().map(|b| &mut b.renderer) else {
            return;
        };
        if crate::gpufence::ready(renderer).is_err() {
            return;
        }
        let cap = *live_previews
            .sheet_cap
            .get_or_insert_with(|| sheet_cap(renderer));
        // No glass in a whole-window picture: it would blur the
        // transparency it is drawn over. The next screen frame begins its
        // own pass.
        let whole = jobs
            .iter()
            .any(|(i, _)| matches!(live_previews.thumbs[*i].show, Show::Whole { .. }));
        if whole && let Some(o) = &output {
            let settings = reprisekit::glass::Settings {
                transparency: false,
                ..config.glass
            };
            glass.begin(
                o,
                settings,
                reprisefx::Quality::default(),
                crate::glass::Hold::No,
            );
        }
        let mut drew = false;
        for sheet in live_previews.sheets.iter_mut() {
            let cells: Vec<(usize, &Shot)> = jobs
                .iter()
                .filter(|(i, _)| live_previews.thumbs[*i].sheet == Some(sheet.id))
                .map(|(i, shot)| (*i, shot))
                .collect();
            if !cells.is_empty() {
                drew |= draw_sheet(
                    renderer,
                    scene,
                    sheet,
                    &mut live_previews.thumbs,
                    &cells,
                    cap,
                );
            }
        }
        for (i, shot) in jobs {
            let t = &mut live_previews.thumbs[i];
            t.marked = false;
            if t.sheet.is_none() {
                drew |= draw(renderer, scene, t, &shot);
            }
            // Wanted or not, the picture is the client's frame: a hidden
            // window's surface gets no other.
            if let Some(o) = &output {
                smithay::desktop::utils::send_frames_surface_tree(
                    &shot.surface,
                    o,
                    now,
                    None,
                    |_, _| Some(o.clone()),
                );
            }
        }
        if drew {
            // A picture over glass changes nothing the glass blurs.
            match shown_on {
                Some(outputs) => {
                    crate::backend::schedule_on(self, &outputs, crate::backend::Ask::Content)
                }
                None => self.redraw_content(),
            }
        }
    }

    /// The outputs showing the thumbs at `indices`: those their wells overlap. None when one is a
    /// whole-window picture, which its host draws wherever it shows it.
    fn thumb_outputs(&self, indices: impl Iterator<Item = usize>) -> Option<Vec<Output>> {
        let mut out: Vec<Output> = Vec::new();
        for i in indices {
            let Show::Well { buffer, .. } = self.live_previews.thumbs[i].show else {
                return None;
            };
            for o in self.scene.outputs_of(buffer) {
                if !out.contains(&o) {
                    out.push(o);
                }
            }
        }
        Some(out)
    }
}

/// How long after `now` a live picture last drawn at `drawn` (None for
/// never) may draw again, `period` apart: zero when it may draw now.
pub fn preview_wait(drawn: Option<Instant>, now: Instant, period: Duration) -> Duration {
    drawn.map_or(Duration::ZERO, |d| {
        period.saturating_sub(now.saturating_duration_since(d))
    })
}

/// Render `shot` into `t`'s texture and show it in `t`'s buffer or slot.
/// True when what shows changed.
fn draw(
    renderer: &mut GlesRenderer,
    scene: &mut crate::compositor::DeskScene,
    t: &mut Thumb,
    shot: &Shot,
) -> bool {
    let Some(Fit {
        size,
        pic,
        mode,
        scale,
    }) = fit_shot(t, shot)
    else {
        return false;
    };
    let (cw, ch) = (shot.rect.width, shot.rect.height);
    let fresh = mode != t.mode || t.target.is_none();
    if fresh {
        t.target = match Target::new(renderer, size.into(), scale) {
            Ok(target) => Some(target),
            Err(e) => {
                log::warn!("live preview: could not size the picture to {size:?}: {e}");
                None
            }
        };
        t.mode = mode;
        if let (Show::Well { buffer, .. }, Some(pic)) = (&t.show, pic) {
            scene.set_position(*buffer, pic.x, pic.y);
            scene.buffer_set_dest_size(*buffer, Some((pic.width, pic.height).into()));
        }
    }
    let Some(target) = t.target.as_mut() else {
        return false;
    };
    // The content subtree's rect is its own; the frame rect is the
    // window's, in layout coordinates.
    let origin = match t.show {
        Show::Well { .. } => match scene.coords(shot.tree) {
            Some((origin, _)) => origin,
            None => return false,
        },
        Show::Whole { .. } => (shot.rect.x, shot.rect.y).into(),
    };
    let view = View::new(Rectangle::new(origin, (cw, ch).into()), scale);
    let elements = scene.render_elements_of(renderer, shot.tree, &view);
    let damage = match target.render(renderer, &elements) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("live preview: {e}");
            return false;
        }
    };
    match &t.show {
        Show::Whole { slot, .. } => {
            if fresh {
                *slot.borrow_mut() = Some(target.texture().clone());
            }
            let changed = fresh || !damage.is_empty();
            // The 3D switcher draws the picture smaller, from its mipmaps.
            if changed && let Err(e) = reprisefx::mipmaps(renderer, target.texture()) {
                log::warn!("live preview: no mipmaps: {e}");
            }
            changed
        }
        Show::Well { buffer, .. } if fresh => {
            scene.buffer_set_texture(*buffer, renderer, target.texture().clone());
            true
        }
        Show::Well { buffer, .. } if !damage.is_empty() => {
            scene.buffer_set_texture_with_damage(
                *buffer,
                renderer,
                target.texture().clone(),
                &damage,
            );
            true
        }
        Show::Well { .. } => false,
    }
}

/// A thumb's picture for the window's current size.
#[derive(Clone, Copy)]
struct Fit {
    /// Device pixels.
    size: (i32, i32),
    /// Where a well's picture sits in its well, logical.
    pic: Option<Rect>,
    mode: (i32, i32, i32, i32),
    /// The window drawn straight at the picture's size.
    scale: f64,
}

fn fit_shot(t: &Thumb, shot: &Shot) -> Option<Fit> {
    let (cw, ch) = (shot.rect.width, shot.rect.height);
    if cw <= 0 || ch <= 0 {
        return None;
    }
    let (size, pic) = match &t.show {
        Show::Well { well, scale, .. } => {
            let pic = fit(cw, ch, *well, 1);
            ((pic.width * scale, pic.height * scale), Some(pic))
        }
        Show::Whole { px, .. } => (*px, None),
    };
    if size.0 <= 0 || size.1 <= 0 {
        return None;
    }
    Some(Fit {
        size,
        pic,
        mode: (size.0, size.1, cw, ch),
        scale: cover_scale(size, (cw, ch)),
    })
}

/// Render the `cells` (thumb index, shot) of `sheet` into its pages, one
/// pass each, and show each cell in its thumb's buffer. A new size of any
/// cell lays the sheet out again. True when what shows changed.
fn draw_sheet(
    renderer: &mut GlesRenderer,
    scene: &mut crate::compositor::DeskScene,
    sheet: &mut Sheet,
    thumbs: &mut [Thumb],
    cells: &[(usize, &Shot)],
    cap: i32,
) -> bool {
    let fits: Vec<Option<Fit>> = cells
        .iter()
        .map(|(i, shot)| fit_shot(&thumbs[*i], shot))
        .collect();
    let fresh = sheet.pages.is_empty()
        || cells.iter().zip(&fits).any(|((i, _), f)| {
            f.is_some_and(|f| f.mode != thumbs[*i].mode || thumbs[*i].cell.is_none())
        });
    if fresh {
        let sizes: Vec<(i32, i32)> = fits.iter().map(|f| f.map_or((0, 0), |f| f.size)).collect();
        let (at, pages) = pack(&sizes, cap);
        let made: Result<Vec<Target>, _> = (pages.iter())
            .map(|&size| Target::new(renderer, size.into(), 1.0))
            .collect();
        sheet.pages = match made {
            Ok(made) => made,
            Err(e) => {
                log::warn!("live preview: could not size the sheet to {pages:?}: {e}");
                Vec::new()
            }
        };
        for (((i, _), f), (page, at)) in cells.iter().zip(&fits).zip(at) {
            let t = &mut thumbs[*i];
            t.cell = f
                .filter(|_| !sheet.pages.is_empty())
                .map(|_| (page, at.into()));
            let (Some(f), Show::Well { buffer, .. }) = (f, &t.show) else {
                continue;
            };
            t.mode = f.mode;
            if let Some(pic) = f.pic {
                scene.set_position(*buffer, pic.x, pic.y);
                scene.buffer_set_dest_size(*buffer, Some((pic.width, pic.height).into()));
            }
        }
    }
    let mut changed = false;
    for (page, target) in sheet.pages.iter_mut().enumerate() {
        let on_page: Vec<(usize, &Shot, Fit, Point<i32, Physical>)> = cells
            .iter()
            .zip(&fits)
            .filter_map(|((i, shot), f)| match (f, thumbs[*i].cell) {
                (Some(f), Some((p, cell))) if p == page => Some((*i, *shot, *f, cell)),
                _ => None,
            })
            .collect();
        if on_page.is_empty() {
            continue;
        }
        changed |= draw_page(renderer, scene, target, thumbs, &on_page, fresh);
    }
    changed
}

/// Render the cells (thumb index, shot, fit, top-left) of one sheet page into
/// `target` and show each in its thumb's buffer. True when what shows changed.
fn draw_page(
    renderer: &mut GlesRenderer,
    scene: &mut crate::compositor::DeskScene,
    target: &mut Target,
    thumbs: &[Thumb],
    cells: &[(usize, &Shot, Fit, Point<i32, Physical>)],
    fresh: bool,
) -> bool {
    let mut elements = Vec::new();
    for &(_, shot, f, cell) in cells {
        let Some((origin, _)) = scene.coords(shot.tree) else {
            continue;
        };
        let (cw, ch) = (shot.rect.width, shot.rect.height);
        let view = View::new(Rectangle::new(origin, (cw, ch).into()), f.scale);
        let crop = Rectangle::new(cell, f.size.into());
        elements.extend(
            scene
                .render_elements_of(renderer, shot.tree, &view)
                .into_iter()
                .filter_map(|e| {
                    let placed = Placed {
                        inner: e,
                        scale: f.scale.into(),
                        offset: cell,
                    };
                    CropRenderElement::from_element(placed, 1.0, crop)
                }),
        );
    }
    let damage = match target.render(renderer, &elements) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("live preview: {e}");
            return false;
        }
    };
    let mut changed = false;
    for &(i, _, f, cell) in cells {
        let Show::Well { buffer, .. } = &thumbs[i].show else {
            continue;
        };
        let rect = Rectangle::<i32, Buffer>::new((cell.x, cell.y).into(), f.size.into());
        if fresh {
            scene.buffer_set_texture(*buffer, renderer, target.texture().clone());
            scene.buffer_set_source_box(*buffer, Some(rect.to_f64()));
            changed = true;
            continue;
        }
        let hit: Vec<_> = damage.iter().filter_map(|d| d.intersection(rect)).collect();
        if !hit.is_empty() {
            scene.buffer_set_texture_with_damage(*buffer, renderer, target.texture().clone(), &hit);
            changed = true;
        }
    }
    changed
}

/// The largest sheet page side: `SHEET_MAX`, or the renderer's texture limit when smaller.
fn sheet_cap(renderer: &mut GlesRenderer) -> i32 {
    let max = renderer.with_context(|gl| {
        let mut max = 0;
        // SAFETY: the context is current and `max` outlives the call.
        unsafe { gl.GetIntegerv(ffi::MAX_TEXTURE_SIZE, &mut max) };
        max
    });
    match max {
        Ok(max) if max > 0 => max.min(SHEET_MAX),
        _ => SHEET_MAX,
    }
}

/// A cell's page and top-left on it.
type Cell = (usize, (i32, i32));

/// The page and top-left of a cell for each picture of `sizes` (device
/// pixels), in rows left to right on pages no wider or taller than `cap`, and
/// each page's size. An empty size takes no room.
fn pack(sizes: &[(i32, i32)], cap: i32) -> (Vec<Cell>, Vec<(i32, i32)>) {
    let mut pages = vec![(1, 1)];
    let (mut x, mut y, mut row) = (0, 0, 0);
    let at = sizes
        .iter()
        .map(|&(w, h)| {
            if w <= 0 || h <= 0 {
                return (0, (0, 0));
            }
            if x > 0 && x + w > cap {
                (x, y, row) = (0, y + row + SHEET_GAP, 0);
            }
            if y > 0 && y + h > cap {
                (x, y, row) = (0, 0, 0);
                pages.push((1, 1));
            }
            let page = pages.len() - 1;
            let at = (x, y);
            row = row.max(h);
            let size = &mut pages[page];
            *size = (size.0.max(x + w), size.1.max(y + row));
            x += w + SHEET_GAP;
            (page, at)
        })
        .collect();
    (at, pages)
}

/// An element drawn at its own `scale` and moved by `offset`, at whatever
/// scale the target asks: cells of one sheet each draw at their own.
struct Placed<E> {
    inner: E,
    scale: Scale<f64>,
    offset: Point<i32, Physical>,
}

impl<E: Element> Element for Placed<E> {
    fn id(&self) -> &Id {
        self.inner.id()
    }
    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }
    fn location(&self, _: Scale<f64>) -> Point<i32, Physical> {
        self.inner.location(self.scale) + self.offset
    }
    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }
    fn transform(&self) -> Transform {
        self.inner.transform()
    }
    fn geometry(&self, _: Scale<f64>) -> Rectangle<i32, Physical> {
        let mut g = self.inner.geometry(self.scale);
        g.loc += self.offset;
        g
    }
    fn damage_since(
        &self,
        _: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.inner.damage_since(self.scale, commit)
    }
    fn opaque_regions(&self, _: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        self.inner.opaque_regions(self.scale)
    }
    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }
    fn kind(&self) -> Kind {
        self.inner.kind()
    }
    fn is_framebuffer_effect(&self) -> bool {
        self.inner.is_framebuffer_effect()
    }
}

impl<R: Renderer, E: RenderElement<R>> RenderElement<R> for Placed<E> {
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), R::Error> {
        self.inner
            .draw(frame, src, dst, damage, opaque_regions, cache)
    }
    fn underlying_storage(&self, renderer: &mut R) -> Option<UnderlyingStorage<'_>> {
        self.inner.underlying_storage(renderer)
    }
    fn capture_framebuffer(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), R::Error> {
        self.inner.capture_framebuffer(frame, src, dst, cache)
    }
}

/// The scale that draws a `client` (w, h) onto a `size` picture with no
/// empty row or column: `fit` floors the picture's shorter side, so the
/// larger of the two ratios fills both and crops under a pixel.
fn cover_scale(size: (i32, i32), client: (i32, i32)) -> f64 {
    let sx = f64::from(size.0) / f64::from(client.0);
    let sy = f64::from(size.1) / f64::from(client.1);
    sx.max(sy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::raster::Rect;

    #[test]
    fn the_picture_fills_both_sides_of_its_box() {
        // A 696x496 terminal in a well whose inside is 106 rows high: the
        // box is 148x106, and 496 rows at 148/696 would cover only 105.47.
        let well = Rect::new(0, 0, 200, 110);
        let pic = fit(696, 496, well, 1);
        assert_eq!((pic.width, pic.height), (148, 106));
        let s = cover_scale((pic.width, pic.height), (696, 496));
        assert!(696.0 * s >= 148.0, "width covered: {}", 696.0 * s);
        assert!(496.0 * s >= 106.0, "height covered: {}", 496.0 * s);
    }

    #[test]
    fn cells_of_a_sheet_lie_apart_in_rows_no_wider_than_the_sheet() {
        let sizes = [(2000, 100), (2000, 150), (0, 0), (2000, 80), (50, 50)];
        let (at, pages) = pack(&sizes, SHEET_MAX);
        assert_eq!(
            at,
            vec![
                (0, (0, 0)),
                (0, (2002, 0)),
                (0, (0, 0)),
                (0, (0, 152)),
                (0, (2002, 152))
            ],
            "two 2000 wide per row, 2 apart; the empty one takes no room"
        );
        assert_eq!(pages, vec![(4002, 232)]);
        assert_eq!(
            pack(&[], SHEET_MAX).1,
            vec![(1, 1)],
            "an empty sheet is still a texture"
        );
    }

    #[test]
    fn a_sheet_taller_than_its_cap_goes_on_to_another_page() {
        for (cell, cap) in [((180, 135), 1024), ((360, 270), SHEET_MAX)] {
            let (at, pages) = pack(&[cell; 200], cap);
            assert!(
                pages.len() > 1,
                "200 {cell:?} cells overflow one {cap} page"
            );
            for &(w, h) in &pages {
                assert!(w <= cap && h <= cap, "page {w}x{h} within {cap}");
            }
            for &(page, (x, y)) in &at {
                let (w, h) = pages[page];
                assert!(
                    x + cell.0 <= w && y + cell.1 <= h,
                    "cell at ({x}, {y}) on its page"
                );
            }
        }
    }

    #[test]
    fn a_cell_of_a_sheet_has_the_pixels_of_a_texture_of_its_own() {
        use smithay::backend::{
            allocator::Fourcc,
            renderer::{ImportMem, element::texture::TextureRenderElement},
        };
        let Some(mut r) = reprisefx::headless::renderer()
            .map_err(|e| eprintln!("skipped: {e}"))
            .ok()
        else {
            return;
        };
        // Red, green / blue, white, B G R A bytes; drawn 3x3 logical at
        // scale 3, so the 6x6 picture crops it.
        let bytes = [
            0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, 255, 255, 255, 255,
        ];
        let texture = r
            .import_memory(&bytes, Fourcc::Argb8888, (2, 2).into(), false)
            .unwrap();
        let context = r.context_id();
        let element = || {
            TextureRenderElement::from_static_texture(
                Id::new(),
                context.clone(),
                (0.0, 0.0),
                texture.clone(),
                1,
                Transform::Normal,
                None,
                None,
                Some((3, 3).into()),
                None,
                Kind::Unspecified,
            )
        };
        let mut own = Target::new(&mut r, (6, 6).into(), 3.0).unwrap();
        own.render(&mut r, &[element()]).unwrap();
        let own = own.read(&mut r).unwrap();

        let cell = Point::from((9, 2));
        let placed = Placed {
            inner: element(),
            scale: 3.0.into(),
            offset: cell,
        };
        let crop = Rectangle::new(cell, (6, 6).into());
        let placed = CropRenderElement::from_element(placed, 1.0, crop).unwrap();
        let mut sheet = Target::new(&mut r, (20, 10).into(), 1.0).unwrap();
        sheet.render(&mut r, &[placed]).unwrap();
        let sheet = sheet.read(&mut r).unwrap();

        for y in 0..10 {
            for x in 0..20 {
                let got = sheet.pixels[y * 20 + x];
                let (cx, cy) = (x as i32 - 9, y as i32 - 2);
                let want = if (0..6).contains(&cx) && (0..6).contains(&cy) {
                    own.pixels[cy as usize * 6 + cx as usize]
                } else {
                    0
                };
                assert_eq!(got, want, "sheet pixel ({x}, {y})");
            }
        }
        assert_ne!(
            own.pixels[0], own.pixels[35],
            "the picture is not one colour"
        );
    }

    #[test]
    fn a_picture_draws_again_a_period_after_its_last_draw() {
        let t0 = Instant::now();
        let period = Duration::from_millis(100);
        let ms = Duration::from_millis;
        assert_eq!(
            preview_wait(None, t0, period),
            Duration::ZERO,
            "never drawn"
        );
        assert_eq!(preview_wait(Some(t0), t0 + ms(30), period), ms(70));
        assert_eq!(preview_wait(Some(t0), t0 + ms(100), period), Duration::ZERO);
        assert_eq!(preview_wait(Some(t0), t0 + ms(250), period), Duration::ZERO);
    }

    #[test]
    fn a_picture_asks_for_frames_only_on_the_outputs_its_well_is_on() {
        let mut h = crate::harness::Harness::new();
        let a = h.output("TEST-1", 1024, 768);
        let b = h.output("TEST-2", 1024, 768);
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "test.app", 300, 200);
        let s = &mut h.state;
        let tree = s.scene.tree_create(s.scene.root());
        let id = s.thumb_create(t.id, tree, Rect::new(0, 0, 100, 80), 1);
        assert!(id.is_some(), "the window has a picture");
        let Show::Well { buffer, .. } = s.live_previews.thumbs[0].show else {
            panic!("a well thumb");
        };
        s.scene.buffer_set_dest_size(buffer, Some((100, 80).into()));
        let b_at = reprisescene::output_rect(&b).expect("TEST-2's place").loc;
        s.scene.set_position(tree, b_at.x + 10, b_at.y + 10);
        assert_eq!(s.thumb_outputs(0..1), Some(vec![b.clone()]));
        s.scene.set_position(tree, b_at.x - 50, b_at.y + 10);
        assert_eq!(s.thumb_outputs(0..1), Some(vec![a, b]), "across the border");
        let slot = TextureSlot::default();
        s.whole_thumb_create(t.id, (100, 80), slot);
        assert_eq!(
            s.thumb_outputs(0..2),
            None,
            "a whole-window picture shows where its host puts it"
        );
    }
}
