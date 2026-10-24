use std::rc::Rc;
use std::time::{Duration, Instant};

use reprisekit::anim::Unroll;
use reprisekit::startpanel::layout::{self, Layout};
use reprisekit::startpanel::{self, Mode, Spot, StartPanel};
use reprisekit::taskbar::{Edge, StartMenuDock};
use reprisekit::textedit::{Preedit, TextSpot};
use reprisescene::Input;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::utils::{Buffer as BufferCoords, Logical, Point, Rectangle};

use crate::barplace::BarPlacement;
use crate::compositor::State;
use crate::config::MenuAnimation;
use crate::glass::{Pane, PaneShape};
use crate::raster::{Canvas, Rect};

use super::StartPanelView;

const FADE_TICK: Duration = Duration::from_millis(16);
const CROSSFADE: Duration = Duration::from_millis(350);

/// What a panel canvas is drawn from, besides the look.
#[derive(Clone, PartialEq)]
pub(super) struct RasterKey {
    pub(super) model: StartPanel,
    pub(super) pressed: Option<Spot>,
    pub(super) caret: bool,
    pub(super) preedit: Option<Preedit>,
    pub(super) scale: i32,
}

/// The last two canvases drawn, so a caret blink back to a drawn state reuses its canvas.
#[derive(Default)]
pub(super) struct Rasters(Vec<(RasterKey, Rc<Canvas>, Option<TextSpot>)>);

impl Rasters {
    pub(super) fn find(&self, key: &RasterKey) -> Option<(Rc<Canvas>, Option<TextSpot>)> {
        let (_, canvas, spot) = self.0.iter().find(|(k, ..)| k == key)?;
        Some((canvas.clone(), spot.clone()))
    }

    pub(super) fn keep(&mut self, key: RasterKey, canvas: Rc<Canvas>, spot: Option<TextSpot>) {
        if self.0.len() == 2 {
            self.0.remove(0);
        }
        self.0.push((key, canvas, spot));
    }
}

pub(super) struct Drawn {
    pub(super) canvas: Rc<Canvas>,
    pub(super) mode: Mode,
    pub(super) picture: Option<usize>,
    /// The canvas shown before, and the box (canvas pixels) where the two differ.
    pub(super) before: Option<(Rc<Canvas>, Rectangle<i32, BufferCoords>)>,
}

impl Drawn {
    /// Where `next` can differ from this canvas: the box from the last change when `next` is the
    /// canvas shown before it, as a caret blink shows; None when it may differ anywhere.
    pub(super) fn differs_within(&self, next: &Rc<Canvas>) -> Option<Rectangle<i32, BufferCoords>> {
        self.before
            .as_ref()
            .filter(|(c, _)| Rc::ptr_eq(c, next))
            .map(|&(_, r)| r)
    }
}

pub(super) struct Crossfade {
    from: Rc<Canvas>,
    rect: Rect,
    start: Instant,
}

pub(super) fn panel_origin(
    dock: StartMenuDock,
    p: &BarPlacement,
    start: (i32, i32),
    area: Rect,
    (w, h): (i32, i32),
) -> (i32, i32) {
    let x = start.0.min(area.right() - w).max(area.x);
    let (dock, bar) = p.start_dock(dock);
    match p.edge {
        Edge::Bottom => {
            let y = dock
                .menu_at(p.edge, (bar.y, bar.bottom()), h)
                .max(area.y + layout::OVERHANG)
                .min(dock.menu_at(Edge::Bottom, (area.bottom(), area.bottom()), h))
                .max(area.y);
            (x, y)
        }
        Edge::Top => {
            let y = dock
                .menu_at(p.edge, (bar.y, bar.bottom()), h)
                .max(dock.menu_at(Edge::Top, (area.y, area.y), h))
                .max(bar.y + layout::OVERHANG)
                .min(area.bottom() - h)
                .max(bar.y);
            (x, y)
        }
        Edge::Left | Edge::Right => {
            let y = start
                .1
                .max(area.y + layout::OVERHANG)
                .min(area.bottom() - h)
                .max(area.y);
            let x = dock.menu_at(p.edge, (bar.x, bar.right()), w);
            (crate::barplace::beside_on_output(x, w, bar, area), y)
        }
    }
}

/// The most height a panel by bar `p` stands in at [`panel_origin`] whole on `area`, with the
/// user picture's overhang on the screen.
pub(super) fn panel_room(dock: StartMenuDock, p: &BarPlacement, area: Rect) -> i32 {
    let (dock, bar) = p.start_dock(dock);
    let (top, bottom) = match p.edge {
        Edge::Bottom => (
            area.y + layout::OVERHANG,
            dock.menu_at(p.edge, (bar.y, bar.bottom()), 0),
        ),
        Edge::Top => (
            dock.menu_at(p.edge, (bar.y, bar.bottom()), 0)
                .max(area.y)
                .max(bar.y + layout::OVERHANG),
            area.bottom(),
        ),
        Edge::Left | Edge::Right => (area.y + layout::OVERHANG, area.bottom()),
    };
    bottom - top
}

pub(super) fn panel_pane(l: &Layout) -> Pane {
    Pane {
        shapes: vec![PaneShape {
            rect: l.panel,
            corner: reprisekit::glass::startpanel::PANEL_CORNER,
            square_bottom: true,
            square_top: false,
        }],
        holes: vec![l.pane],
    }
}

pub(super) fn takes_pointer(l: &Layout) -> impl Fn(Point<f64, Logical>) -> bool + 'static {
    let (panel, picture, origin) = (l.panel, l.picture, (l.canvas.x, l.canvas.y));
    move |p| {
        let (x, y) = (p.x.floor() as i32 + origin.0, p.y.floor() as i32 + origin.1);
        panel.contains(x, y) || picture.is_some_and(|r| r.contains(x, y))
    }
}

/// Shows the unrolling panel `v` at `t` (0 to 1) of its opening; at 1 it stands whole.
fn unroll_frame(v: &mut StartPanelView, scene: &mut crate::compositor::DeskScene, t: f32) {
    let Some(from) = v.unroll else {
        return;
    };
    let e = v.extent;
    let Unroll { shift, clip } = Unroll::at(from, (e.width, e.height), t);
    let done = t >= 1.0;
    v.shift = if done { (0, 0) } else { shift };
    scene.set_position(v.tree, v.at.0 + v.shift.0, v.at.1 + v.shift.1);
    scene.set_clip(
        v.tree,
        (!done).then(|| {
            Rectangle::new(
                (e.x + clip.x, e.y + clip.y).into(),
                (clip.width, clip.height).into(),
            )
        }),
    );
    if done {
        v.unroll = None;
    }
}

impl State {
    fn arm_start_panel_crossfade(&mut self) {
        if self
            .start_panel
            .as_ref()
            .is_none_or(|v| v.crossfade_timer.is_some())
        {
            return;
        }
        let token = self
            .handle
            .insert_source(Timer::from_duration(FADE_TICK), |_, _, state| {
                state.redraw_start_panel();
                match state.start_panel.as_mut() {
                    Some(v) if !v.crossfades.is_empty() => TimeoutAction::ToDuration(FADE_TICK),
                    Some(v) => {
                        v.crossfade_timer = None;
                        TimeoutAction::Drop
                    }
                    None => TimeoutAction::Drop,
                }
            });
        match (token, self.start_panel.as_mut()) {
            (Ok(t), Some(v)) => v.crossfade_timer = Some(t),
            (Ok(t), None) => self.handle.remove(t),
            (Err(e), _) => log::warn!("start panel crossfade timer: {}", e.error),
        }
    }

    pub(super) fn arm_start_panel_fade(&mut self) {
        let token = self
            .handle
            .insert_source(Timer::from_duration(FADE_TICK), |_, _, state| {
                let fade = state.config.theme.metrics.menu_fade_ms.max(1) as f32;
                let Some(v) = state.start_panel.as_mut() else {
                    return TimeoutAction::Drop;
                };
                let t = (v.opened.elapsed().as_secs_f32() * 1000.0 / fade).min(1.0);
                if t >= 1.0 {
                    v.fade = None;
                }
                let (tree, output) = (v.tree, v.output.clone());
                if v.unroll.is_some() {
                    unroll_frame(v, &mut state.scene, t);
                } else {
                    state.scene.set_opacity(tree, t);
                }
                crate::backend::schedule(state, &output);
                if t >= 1.0 {
                    TimeoutAction::Drop
                } else {
                    TimeoutAction::ToDuration(FADE_TICK)
                }
            });
        match (token, self.start_panel.as_mut()) {
            (Ok(t), Some(v)) => v.fade = Some(t),
            (Ok(t), None) => self.handle.remove(t),
            (Err(e), _) => log::warn!("start panel fade timer: {}", e.error),
        }
    }

    pub fn redraw_start_panel(&mut self) {
        let Some(v) = self.start_panel.as_ref() else {
            return;
        };
        let Some(start) = self.bar(v.bar).map(|b| b.layout.start) else {
            return;
        };
        let p = self.bar_placement(&v.output);
        let area = self.usable_area(Some(&v.output));
        let dock = self.config.theme.taskbar_shape().start_menu;
        let room = (area.width, panel_room(dock, &p, area));
        let text = self.config.theme.text;
        if let Some(v) = self.start_panel.as_mut() {
            v.model.fit(text, room);
        }
        let Some(v) = self.start_panel.as_ref() else {
            return;
        };
        let lay = layout::layout(&v.model);
        let at = panel_origin(
            dock,
            &p,
            (p.shown.x + start.x, p.shown.y + start.y),
            area,
            (lay.panel.width, lay.panel.height),
        );
        let scale = self.draw_scale_at(Some(&v.output)).max(1);
        let pane = crate::glass::wanted(&self.config).then(|| panel_pane(&lay));
        let sl = lay.scaled(scale);
        let fades = self.config.menu_opening() != MenuAnimation::None;
        let ms = self.config.theme.metrics.menu_fade_ms.max(1) as f32;
        let key = RasterKey {
            model: v.model.clone(),
            pressed: v.pressed,
            caret: self.caret.on,
            preedit: self.ime_field_preedit(crate::caret::Editor::StartPanel),
            scale,
        };
        let (raster, spot) = match v.rasters.find(&key) {
            Some(drawn) => drawn,
            None => {
                let theme = self.config.theme.scaled(scale);
                let mut canvas = Canvas::new(sl.canvas.width as u32, sl.canvas.height as u32);
                canvas.set_preedit(key.preedit.clone());
                reprisekit::interfaces::draw_start_panel_backed(
                    &mut self.start_backing,
                    &mut canvas,
                    &theme,
                    &self.config.glass,
                    self.start_panel_text.for_output(scale),
                    &v.model,
                    &sl,
                    v.pressed,
                    self.caret.on,
                );
                let spot = canvas.take_spot();
                let raster = Rc::new(canvas);
                if let Some(v) = self.start_panel.as_mut() {
                    v.rasters.keep(key, raster.clone(), spot.clone());
                }
                (raster, spot)
            }
        };
        let origin = (at.0 + lay.canvas.x, at.1 + lay.canvas.y);
        self.ime_field_drawn(crate::caret::Editor::StartPanel, spot, origin, scale);
        let State {
            start_panel,
            scene,
            glass,
            ..
        } = self;
        let Some(v) = start_panel.as_mut() else {
            return;
        };
        v.at = at;
        v.extent = lay.canvas;
        scene.set_position(v.tree, at.0, at.1);
        if v.unroll.is_some() {
            unroll_frame(
                v,
                scene,
                (v.opened.elapsed().as_secs_f32() * 1000.0 / ms).min(1.0),
            );
        }
        scene.set_position(v.buffer, lay.canvas.x, lay.canvas.y);
        scene.set_input(v.buffer, Input::Test(Box::new(takes_pointer(&lay))));
        v.backdrop.set(scene, glass, v.glass_tree, pane);
        let (mode, picture) = (v.model.mode(), v.model.picture());
        let now = Instant::now();
        if fades && let Some(old) = &v.drawn {
            let inside =
                |r: Rect| Rect::new(r.x - sl.canvas.x, r.y - sl.canvas.y, r.width, r.height);
            let mut rects = Vec::new();
            if matches!(
                (old.mode, mode),
                (Mode::Home, Mode::Programs) | (Mode::Programs, Mode::Home)
            ) {
                rects.push(inside(sl.pane));
            }
            if old.picture != picture
                && let Some(r) = sl.picture
            {
                rects.push(inside(r));
            }
            v.crossfades.extend(rects.into_iter().map(|rect| Crossfade {
                from: old.canvas.clone(),
                rect,
                start: now,
            }));
        }
        let size = (raster.width, raster.height);
        v.crossfades
            .retain(|f| now - f.start < CROSSFADE && (f.from.width, f.from.height) == size);
        let canvas = if v.crossfades.is_empty() {
            raster
        } else {
            let mut canvas = (*raster).clone();
            for f in &v.crossfades {
                let t = (now - f.start).as_secs_f32() / CROSSFADE.as_secs_f32();
                canvas.mix(&f.from, f.rect, 1.0 - t);
            }
            Rc::new(canvas)
        };
        let shown = v.drawn.take();
        let (w, h) = (canvas.width as i32, canvas.height as i32);
        let changed = match shown.as_ref().and_then(|d| d.differs_within(&canvas)) {
            Some(within) => scene.buffer_set_canvas_within(v.buffer, &canvas.pixels, w, h, within),
            None => scene.buffer_set_canvas(v.buffer, &canvas.pixels, w, h),
        };
        scene.buffer_set_dest_size(v.buffer, Some((w / scale, h / scale).into()));
        v.drawn = Some(Drawn {
            canvas,
            mode,
            picture,
            before: shown.map(|d| (d.canvas, changed)),
        });
        let output = v.output.clone();
        if !v.crossfades.is_empty() {
            self.arm_start_panel_crossfade();
        }
        crate::backend::schedule(self, &output);
        crate::access::touch(self);
    }

    /// The look or the text size changed under the open panel: draw it again from scratch.
    pub fn retheme_start_panel(&mut self) {
        self.start_backing = Default::default();
        let text = self.config.theme.text;
        if let Some(v) = self.start_panel.as_mut() {
            v.model.text = text;
            v.rasters = Rasters::default();
        }
        self.redraw_start_panel();
    }

    pub fn start_panel_access(&self) -> Option<reprisekit::access::Tree> {
        use reprisekit::access::{ROOT, accesskit::Role};
        let v = self.start_panel.as_ref()?;
        let lay = layout::layout(&v.model);
        let rect = Rect::new(v.at.0, v.at.1, lay.panel.width, lay.panel.height);
        let mut tree = crate::access::root(Role::Window, "Start menu", rect);
        let focus = startpanel::access::access(&v.model, &lay, &mut tree, ROOT);
        tree.set_focus(focus);
        Some(tree)
    }
}
