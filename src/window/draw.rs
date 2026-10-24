use std::rc::Rc;

use reprisekit::classic::frame::{FrameGeometry, FrameHit, FrameState, frame_canvas_size};
use reprisekit::controls::captiontabs::{self, CaptionTabs, TabEntry};
use reprisekit::glass::{Material, shadow, tint};
use reprisescene::{BufferId, Input, NodeId, TreeId};
use smithay::utils::{Logical, Rectangle};

use crate::compositor::{DeskScene, State};
use crate::glass::{frame_pane, sheet_pane};
use crate::output::upload_scaled;
use crate::raster::{Canvas, Rect};

use super::{Window, WindowId};

/// Glass shadow pieces per (scale, active, square corners), drawn once and shared; each window's
/// nodes take their own pixel copy.
#[derive(Default)]
pub struct ShadowCache(pub(super) Vec<(ShadowKey, Rc<[Canvas; 8]>)>);

pub(super) type ShadowKey = (i32, bool, bool);

impl ShadowCache {
    pub fn pieces(&mut self, scale: i32, active: bool, square: bool) -> Rc<[Canvas; 8]> {
        let key = (scale, active, square);
        if let Some((_, p)) = self.0.iter().find(|(k, _)| *k == key) {
            return p.clone();
        }
        let p: Rc<[Canvas; 8]> = Rc::new(std::array::from_fn(|i| {
            if square {
                reprisekit::glass::square::shadow_piece(active, i, scale)
            } else {
                shadow::piece(active, i, scale)
            }
        }));
        self.0.push((key, p.clone()));
        p
    }
}

/// The frames `redraw_frame` put off inside `State::batch_frames`, each once, in the order first
/// asked for.
#[derive(Default)]
pub struct FrameBatch(Option<Vec<WindowId>>);

impl FrameBatch {
    /// Start a batch; false if one is open already (the outer one draws).
    pub(super) fn begin(&mut self) -> bool {
        if self.0.is_some() {
            return false;
        }
        self.0 = Some(Vec::new());
        true
    }

    /// Whether `w`'s redraw waits for the batch's end.
    pub(super) fn put_off(&mut self, w: WindowId) -> bool {
        let Some(batch) = self.0.as_mut() else {
            return false;
        };
        if !batch.contains(&w) {
            batch.push(w);
        }
        true
    }

    pub(super) fn end(&mut self) -> Vec<WindowId> {
        self.0.take().unwrap_or_default()
    }
}

/// Whether (`x`, `y`) takes the pointer: clear pixels of a styled caption's cut corner pass
/// through.
fn frame_takes(
    look: &reprisekit::paint::Look,
    geom: &FrameGeometry,
    active: bool,
    maximized: bool,
    x: i32,
    y: i32,
) -> bool {
    match look.style() {
        Some(style) => {
            reprisekit::style::frame::frame_opaque_at(style, geom, active, maximized, x, y)
        }
        None => true,
    }
}

impl State {
    pub fn redraw_frame(&mut self, w: WindowId) {
        if self.frame_batch.put_off(w) {
            return;
        }
        self.place_backdrop(w);
        crate::access::frame_changed(self);
        if self.is_sheet(w) {
            let scale = self.draw_scale_for_window(w);
            if let Some(win) = self.win_mut(w) {
                win.frame_scale = scale;
            }
            self.place_shadow(w);
            self.draw_sheet(w);
            return;
        }
        self.unsheet(w);
        let Some(win) = self.win(w) else {
            return;
        };
        if !win.has_frame() {
            self.place_shadow(w);
            return;
        }
        let scale = self.draw_scale_for_window(w);
        if let Some(win) = self.win_mut(w) {
            win.frame_scale = scale;
        }
        self.place_shadow(w);
        self.draw_strips(w, false);
    }

    /// Run `f` with `redraw_frame` put off: each frame it asked for draws once afterwards, in its
    /// final state.
    pub fn batch_frames(&mut self, f: impl FnOnce(&mut Self)) {
        let outer = self.frame_batch.begin();
        f(self);
        if outer {
            for w in self.frame_batch.end() {
                self.redraw_frame(w);
            }
        }
    }

    /// Whether `w` shows as a sheet: peek hides it, it is not fullscreen, and the look draws
    /// translucent glass with a sheet.
    pub(crate) fn is_sheet(&self, w: WindowId) -> bool {
        self.peek.hides(w)
            && self.win(w).is_some_and(|x| !x.fullscreen)
            && crate::glass::wanted(&self.config)
            && reprisekit::look::sheet_edge(&self.config.theme).is_some()
    }

    /// Draw `w` as a sheet over its whole rect: edges in the four strips, plain glass stretched in
    /// between; the pointer passes through.
    fn draw_sheet(&mut self, w: WindowId) {
        self.thumbs_frame(w);
        let Some(scale) = self.win(w).map(|x| x.frame_scale.max(1)) else {
            return;
        };
        let theme = self.config.theme.scaled(scale);
        let Some(edge) = reprisekit::look::sheet_edge(&theme) else {
            return;
        };
        let Some(win) = self.win_mut(w) else {
            return;
        };
        win.sheeted = true;
        let (frame, middle) = (win.frame, win.sheet);
        let (fw, fh) = win.geom.total;
        let e = edge.min(fw / 2).min(fh / 2);
        let pieces = [
            Rect::new(0, 0, fw, e),
            Rect::new(0, e, e, fh - 2 * e),
            Rect::new(fw - e, e, e, fh - 2 * e),
            Rect::new(0, fh - e, fw, e),
        ];
        let whole = |at: (i32, i32)| Rect::new(-at.0, -at.1, fw * scale, fh * scale);
        for (sb, r) in frame.into_iter().zip(pieces) {
            if r.is_empty() {
                self.scene.set_enabled(sb, false);
                continue;
            }
            let mut c = Canvas::new((r.width * scale) as u32, (r.height * scale) as u32);
            reprisekit::look::draw_sheet(&mut c, &theme, whole((r.x * scale, r.y * scale)));
            upload_scaled(&mut self.scene, sb, &c, scale);
            self.scene.set_position(sb, r.x, r.y);
            self.scene.set_input(sb, Input::Passthrough);
            self.scene.set_enabled(sb, true);
        }
        let inside = Rect::new(e, e, fw - 2 * e, fh - 2 * e);
        if inside.is_empty() {
            self.scene.set_enabled(middle, false);
            return;
        }
        let mut c = Canvas::new(1, 1);
        reprisekit::look::draw_sheet(&mut c, &theme, whole((fw * scale / 2, fh * scale / 2)));
        self.scene.buffer_set_canvas(middle, &c.pixels, 1, 1);
        self.scene
            .buffer_set_dest_size(middle, Some((inside.width, inside.height).into()));
        self.scene.set_position(middle, inside.x, inside.y);
        self.scene.set_enabled(middle, true);
    }

    /// Restore the frame nodes `draw_sheet` took (`draw_strips` sets the caption's and bottom's hit
    /// tests).
    fn unsheet(&mut self, w: WindowId) {
        let Some(win) = self.win_mut(w).filter(|x| x.sheeted) else {
            return;
        };
        win.sheeted = false;
        let (middle, frame, on) = (win.sheet, win.frame, win.has_frame());
        self.scene.set_enabled(middle, false);
        for sb in frame {
            self.scene.set_enabled(sb, on);
            self.scene.set_input(sb, Input::All);
        }
    }

    /// The 3D switcher's cards came up or went: while up, frames on its output draw inactive and
    /// their shadows hide (`carded`).
    pub(crate) fn cards_changed(&mut self) {
        let ids: Vec<(WindowId, bool)> = (self.windows.iter())
            .map(|x| (x.id, x.minimized || Some(x.id) == self.focused))
            .collect();
        for (w, redraw) in ids {
            if redraw {
                self.redraw_frame(w);
            } else {
                self.place_shadow(w);
            }
        }
    }

    pub fn redraw_caption(&mut self, w: WindowId) {
        if self.is_sheet(w) {
            return;
        }
        if self.win(w).is_some_and(Window::has_frame) {
            self.draw_strips(w, true);
            self.redraw_window(w);
        }
        let shown = self.tabs.groups.shown_of(w);
        if shown != w && self.win(shown).is_some_and(Window::has_frame) {
            self.draw_strips(shown, true);
            self.redraw_window(shown);
        }
    }

    /// Ask for frames on the outputs `w`'s tree is on, for an edit of its frame.
    pub fn redraw_window(&mut self, w: WindowId) {
        let Some(tree) = self.win(w).map(|x| x.tree) else {
            return;
        };
        let outputs = self.scene.outputs_of(tree);
        crate::backend::schedule_on(self, &outputs, crate::backend::Ask::Scene);
    }

    fn draw_strips(&mut self, w: WindowId, only_top: bool) {
        self.thumbs_frame(w);
        let Some(scale) = self.win(w).map(|x| x.frame_scale) else {
            return;
        };
        let theme = self.config.theme.scaled(scale);
        let icon = {
            let State {
                windows,
                icons,
                app_index,
                ..
            } = self;
            windows
                .iter()
                .find(|x| x.id == w)
                .and_then(|win| crate::icon::window_icon(icons, app_index, win, theme.icon_size()))
        };
        let strip = self.strip_for(w, theme.icon_size());
        let Some(win) = self.win(w) else {
            return;
        };
        let now = std::time::Instant::now();
        let geom = win.geom.scaled(scale);
        let layout = self.layout_box(None);
        let limit = (layout.width * scale, layout.height * scale);
        let title = win.title();
        let active = win.is_active(self.focused) && !self.carded(w);
        let state = FrameState {
            title: &title,
            active,
            maximized: win.maximized,
            pressed: win.pressed,
            glow: reprisekit::frame::Glow(std::array::from_fn(|i| win.glow[i].level(now))),
            icon: icon.as_deref(),
            tabbed: strip.is_some(),
        };
        let tabs = strip.as_ref().map(|st| CaptionTabs {
            tabs: (st.tabs.iter())
                .map(|t| TabEntry {
                    title: &t.title,
                    icon: t.icon.as_deref(),
                })
                .collect(),
            selected: st.selected,
            hot: st.hot,
            held: st.held,
            ghost: st.ghost,
        });
        let strips = win.geom.strips();
        let count = if only_top { 1 } else { strips.len() };
        let mut drawn = Vec::new();
        for (i, r) in geom.strips().iter().enumerate().take(count) {
            if r.is_empty() {
                drawn.push((i, None));
                continue;
            }
            let (cw, ch) = frame_canvas_size((r.width, r.height), limit);
            let mut canvas = Canvas::new(cw, ch);
            let at = geom.translated(r.x, r.y);
            reprisekit::look::draw_frame(
                &mut canvas,
                &theme,
                self.text.for_output(theme.scale),
                self.caption_text.for_output(theme.scale),
                &at,
                &state,
            );
            if let Some(tabs) = &tabs {
                captiontabs::draw(
                    &mut canvas,
                    &theme,
                    self.text.for_output(theme.scale),
                    self.caption_text.for_output(theme.scale),
                    &at,
                    tabs,
                    active,
                );
            }
            drawn.push((i, Some(canvas)));
        }
        let (frame, logical, maximized) = (win.frame, win.geom, win.maximized);
        for (i, canvas) in drawn {
            let sb = frame[i];
            let Some(canvas) = canvas else {
                self.scene.buffer_clear(sb);
                continue;
            };
            upload_scaled(&mut self.scene, sb, &canvas, scale);
            self.scene.set_position(sb, strips[i].x, strips[i].y);
            if i == 0 || i == 3 {
                let look = self.config.theme.look.clone();
                let geom = logical;
                let at = strips[i];
                self.scene.set_input(
                    sb,
                    Input::Test(Box::new(move |p| {
                        frame_takes(
                            &look,
                            &geom,
                            active,
                            maximized,
                            p.x.floor() as i32 + at.x,
                            p.y.floor() as i32 + at.y,
                        )
                    })),
                );
            }
        }
    }

    fn place_backdrop(&mut self, w: WindowId) {
        let sheet = self.is_sheet(w);
        self.place_client_glass(w, sheet);
        let pane = match self.config.theme.glass() {
            Some(g) if self.config.glass.transparency => self
                .win(w)
                .filter(|win| sheet || win.has_frame())
                .map(|win| match sheet {
                    true => sheet_pane(win.geom.total, g.corner),
                    false => {
                        let mut p = frame_pane(&win.geom, g.corner, win.maximized);
                        p.holes = client_holes(win.geom.client, &win.blur);
                        p
                    }
                }),
            _ => None,
        };
        let over = self.config.glass.material == Material::Molten;
        let active = self.win(w).is_some_and(|x| x.is_active(self.focused)) && !self.carded(w);
        let surface = match sheet || active {
            true => tint::Surface::Active,
            false => tint::Surface::Inactive,
        };
        let State {
            scene,
            windows,
            glass,
            ..
        } = self;
        if let Some(win) = windows.iter_mut().find(|x| x.id == w) {
            win.glass.set_surface(surface);
            win.glass.set(scene, glass, win.backdrop, pane);
            stack_shadow(scene, &win.shadow, win.backdrop, over);
        }
    }

    /// Lay the frame's glass under the client's blur region when the look has it and the window
    /// shows its frame; the client is then left out of what glass blurs, so its own glass reads
    /// what lies behind the window.
    fn place_client_glass(&mut self, w: WindowId, sheet: bool) {
        let active = self.win(w).is_some_and(|x| x.is_active(self.focused)) && !self.carded(w);
        let scale = self.draw_scale_for_window(w);
        let theme = crate::theme::Theme {
            active,
            ..self.config.theme.scaled(scale)
        };
        let opens = reprisekit::look::client_glass(&theme, active).is_some();
        let State { scene, windows, .. } = self;
        let Some(win) = windows.iter_mut().find(|x| x.id == w) else {
            return;
        };
        let rects: Vec<Rect> = match opens {
            true if win.has_frame() && !sheet => win.blur.clone(),
            _ => Vec::new(),
        };
        while win.client_glass.len() > rects.len() {
            if let Some(b) = win.client_glass.pop() {
                scene.destroy(b);
            }
        }
        while win.client_glass.len() < rects.len() {
            let b = scene.buffer_create(win.chrome);
            scene.set_input(b, Input::Passthrough);
            win.client_glass.push(b);
        }
        let c = win.geom.client;
        for (&b, r) in win.client_glass.iter().zip(&rects) {
            let mut canvas = Canvas::new((r.width * scale) as u32, (r.height * scale) as u32);
            let band = Rect::new(-r.x, -r.y, c.width, r.bottom()).scaled(scale);
            let client_h = c.height * scale;
            reprisekit::look::draw_client_glass(&mut canvas, &theme, band, client_h, win.maximized);
            upload_scaled(scene, b, &canvas, scale);
            scene.set_position(b, c.x + r.x, c.y + r.y);
        }
        let leave_out = !rects.is_empty() && crate::glass::wanted(&self.config);
        scene.set_in_backdrop(win.content, !leave_out);
    }

    /// Show the shadow under Glass; none for maximized, frameless or non-Glass windows, and a sheet
    /// shows the unfocused one.
    fn place_shadow(&mut self, w: WindowId) {
        let spec = self.config.theme.glass();
        let glass = spec.is_some();
        let square = spec.is_some_and(|g| g.corner.is_empty());
        let focused = self.focused;
        let Some(win) = self.win(w) else {
            return;
        };
        let shadow = win.shadow;
        if !glass && shadow.iter().all(|&sb| !self.scene.enabled(sb)) {
            return;
        }
        let sheet = self.is_sheet(w);
        let on = glass && (win.has_frame() || sheet) && !self.carded(w);
        let (fw, fh) = win.geom.total;
        let active = win.is_active(focused) && !sheet;
        let placed = shadow::place(Rect::new(0, 0, fw, fh), active, win.maximized || !on);
        let scale = win.frame_scale.max(1);
        let key = (scale, active, square);
        // Upload only on a key change; all eight repaint, hidden ones too, since the key is kept
        // for all.
        let repaint = on && win.shadow_key != Some(key);
        let pieces = repaint.then(|| self.shadows.pieces(scale, active, square));
        for (i, p) in placed.iter().enumerate() {
            let sb = shadow[i];
            if let Some(pieces) = &pieces {
                let piece = &pieces[i];
                self.scene.buffer_set_canvas(
                    sb,
                    &piece.pixels,
                    piece.width as i32,
                    piece.height as i32,
                );
            }
            if !on || p.dest.is_empty() {
                self.scene.set_enabled(sb, false);
                continue;
            }
            let s = f64::from(scale);
            self.scene.buffer_set_source_box(
                sb,
                Some(Rectangle::new(
                    (f64::from(p.src.x) * s, f64::from(p.src.y) * s).into(),
                    (f64::from(p.src.width) * s, f64::from(p.src.height) * s).into(),
                )),
            );
            self.scene
                .buffer_set_dest_size(sb, Some((p.dest.width, p.dest.height).into()));
            self.scene.set_position(sb, p.dest.x, p.dest.y);
            self.scene.set_enabled(sb, true);
        }
        if repaint && let Some(win) = self.win_mut(w) {
            win.shadow_key = Some(key);
        }
    }

    pub fn refit_scale(&mut self, w: WindowId) {
        let Some(win) = self.win(w) else {
            return;
        };
        if !win.mapped || !win.has_frame() {
            return;
        }
        if self.draw_scale_for_window(w) != win.frame_scale {
            self.redraw_frame(w);
        }
    }

    /// Light the caption button `hit` is on and put out any other window's; fades only when the hot
    /// button changes. Classic lights nothing.
    pub fn frame_hover(&mut self, window: Option<WindowId>, hit: FrameHit) {
        let styled = self.config.theme.look.style().is_some();
        let want = match hit {
            FrameHit::Button(b) if styled && window.is_some() => Some(b),
            _ => None,
        };
        if let Some(old) = self.hot_frame
            && Some(old) != window
            && self.win(old).is_some_and(|o| o.hot.is_some())
        {
            if let Some(o) = self.win_mut(old) {
                o.hot = None;
            }
            self.glow_to_hot(old);
        }
        self.hot_frame = None;
        let Some(window) = window else {
            return;
        };
        if self.win(window).is_some_and(|x| x.hot != want) {
            if let Some(x) = self.win_mut(window) {
                x.hot = want;
            }
            self.glow_to_hot(window);
        }
        if want.is_some() {
            self.hot_frame = Some(window);
        }
    }
}

/// Stack the shadow pieces just under the glass `backdrop` or (`over`) just above it, frame strips
/// on top. Molten's lens reads past the frame edge, so a shadow under the glass bends into the
/// caption as a dark line.
/// The parts of `client` (frame coordinates) the frame's glass leaves clear: all of it but the
/// `blur` rects (client coordinates).
pub(super) fn client_holes(client: Rect, blur: &[Rect]) -> Vec<Rect> {
    let r = |x: Rect| Rectangle::<i32, Logical>::new((x.x, x.y).into(), (x.width, x.height).into());
    let glass = blur
        .iter()
        .map(|b| r(Rect::new(client.x + b.x, client.y + b.y, b.width, b.height)));
    r(client)
        .subtract_rects(glass)
        .into_iter()
        .map(|h| Rect::new(h.loc.x, h.loc.y, h.size.w, h.size.h))
        .collect()
}

pub(super) fn stack_shadow(
    scene: &mut DeskScene,
    shadow: &[BufferId; 8],
    backdrop: TreeId,
    over: bool,
) {
    let Some(parent) = scene.parent(backdrop) else {
        return;
    };
    let children = scene.children(parent);
    let at = |n: NodeId| children.iter().position(|c| *c == n);
    let (Some(b), Some(first)) = (at(backdrop.into()), at(shadow[0].into())) else {
        return;
    };
    if (first > b) == over {
        return;
    }
    if over {
        let mut below: NodeId = backdrop.into();
        for &sb in shadow {
            scene.place_above(sb, below);
            below = sb.into();
        }
    } else {
        for &sb in shadow {
            scene.place_below(sb, backdrop);
        }
    }
}
