//! The 3D switcher (the `flip` bind, Super+Tab) on the scene: `reprisekit::controls::flip` lays the stack out, `reprisefx::flip`
//! draws each card. It lists what Alt+Tab lists plus the desktop, and shows only once the bind is held `OPEN_DELAY_MS`.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use reprisekit::anim::{Ease, Tween};
use reprisekit::controls::flip::{self, Card, GEOMETRY, Homography};
use reprisekit::switcher;
use reprisescene::{CustomElement, CustomNode, Input, RectId, TreeId};
use smithay::{
    backend::{
        input::InputTime,
        renderer::{Texture, gles::GlesRenderer},
    },
    output::Output,
    reexports::calloop::{
        RegistrationToken,
        timer::{TimeoutAction, Timer},
    },
    utils::{Logical, Physical, Point, Rectangle, Scale, Size},
};

use crate::{
    compositor::{SceneOwner, State},
    glass::GlassCtx,
    livepreview::{TextureSlot, ThumbId},
    offscreen::Target,
    raster::Rect,
    switcherview::Entry,
    taskbar::TaskbarHit,
    window::WindowId,
};

const DIM: f32 = 0.5;

const OPEN_DELAY_MS: u64 = 190;
/// Milliseconds the cards take to fly out of the desktop into the stack,
/// eased out, as the dim fades in evenly.
const OPEN_MS: u32 = 250;
const CLOSE_MS: u32 = 250;
const ROLL_MS: u32 = 150;
const QUEUE_MS: u32 = 200;
const PICK_MS: u32 = 350;

/// A 3D switcher: open (`SwitcherView::stack`), or flying back after a
/// commit or cancel (`SwitcherView::leaving`). The entries, the selected
/// one in front, the bind's modifier mask (releasing it commits) and the
/// stack's scene tree.
pub struct Stack {
    pub entries: Vec<Entry>,
    pub selected: usize,
    pub mods: u32,
    output: Output,
    pub area: Rect,
    /// In `layers.overlay` at the output's corner, tagged
    /// `SceneOwner::Switcher`: the dim, which takes the pointer for the
    /// whole output while the stack is open, then the cards' node.
    tree: TreeId,
    dim: RectId,
    /// By entry: the card's size, logical (a window's frame, a dialog,
    /// the output for the desktop).
    sizes: Vec<(i32, i32)>,
    /// By entry: where it lies on the desktop, in the output's own
    /// coordinates; None for one not there (minimized, the desktop).
    flats: Vec<Option<Rect>>,
    /// The entry on top of the desktop: the focused one as the stack
    /// opens, the picked one as it closes.
    top: usize,
    pics: Vec<Pic>,
    /// The scroll position: the entry at the front. Its target runs on
    /// past either end, so a step always rolls its own way.
    roll: Tween,
    out: Tween,
    shade: Tween,
    reveal: Option<RegistrationToken>,
    pick: bool,
    /// The drawn cards, back to front, laid out in the output's own
    /// coordinates, where they are drawn now.
    pub cards: Vec<Card>,
    draws: Rc<RefCell<Vec<Draw>>>,
    hidden: Vec<TreeId>,
    stepping: bool,
    wheel: i32,
    /// When the desktop's live picture last drew, and the timer that asks
    /// for the frame that draws it again, `[previews] fps` on.
    desk_drawn: Option<Instant>,
    desk_timer: Option<RegistrationToken>,
}

/// One entry's picture: the card's element Id, kept for the stack's life,
/// and the texture it draws.
struct Pic {
    card: Rc<reprisefx::FlipCard>,
    slot: TextureSlot,
    thumb: Option<ThumbId>,
    still: bool,
    /// The desktop's live picture while its card is drawn; `slot` shows
    /// its texture.
    desk: Option<Target>,
}

impl Stack {
    pub fn revealed(&self) -> bool {
        self.reveal.is_none()
    }

    fn moving(&self, now: Instant) -> bool {
        !self.out.settled(now) || !self.roll.settled(now) || !self.shade.settled(now)
    }
}

impl Pic {
    fn new() -> Pic {
        Pic {
            card: Rc::new(reprisefx::FlipCard::new()),
            slot: TextureSlot::default(),
            thumb: None,
            still: false,
            desk: None,
        }
    }
}

/// One drawn card: its element Id and picture, and its map from the
/// picture (logical, `size`) to the output.
struct Draw {
    card: Rc<reprisefx::FlipCard>,
    slot: TextureSlot,
    to_screen: Homography,
    size: (i32, i32),
    alpha: f32,
}

/// Every drawn card on the scene: at the stack tree's origin, covering
/// the output, taking no pointer (the dim under it does).
struct CardsNode {
    ctx: Rc<GlassCtx>,
    draws: Rc<RefCell<Vec<Draw>>>,
    extent: Size<i32, Logical>,
}

impl CustomNode<GlesRenderer> for CardsNode {
    fn size(&self) -> Size<i32, Logical> {
        self.extent
    }

    fn elements(
        &self,
        _renderer: &mut GlesRenderer,
        location: Point<f64, Physical>,
        scale: Scale<f64>,
        alpha: f32,
    ) -> Vec<CustomElement<GlesRenderer>> {
        let Some(fx) = self.ctx.flip_fx() else {
            return Vec::new();
        };
        let view = Homography::affine(scale.x, scale.y, location.x, location.y);
        let draws = self.draws.borrow();
        let mut out = Vec::with_capacity(draws.len());
        for d in draws.iter() {
            let texture = d.slot.borrow();
            let Some(texture) = texture.as_ref() else {
                continue;
            };
            let t = texture.size();
            let (w, h) = (f64::from(d.size.0), f64::from(d.size.1));
            let texels = Homography::affine(w / f64::from(t.w), h / f64::from(t.h), 0.0, 0.0);
            let m = view.after(&d.to_screen).after(&texels);
            out.extend(
                (d.card)
                    .element(&fx, texture, m, d.alpha * alpha, true)
                    .map(CustomElement::new),
            );
        }
        out
    }
}

impl State {
    /// Whether the `flip` bind opens the 3D switcher: `[switcher] flip`
    /// under Glass. Otherwise it opens the Alt+Tab panel.
    pub fn flip_wanted(&self) -> bool {
        self.config.switcher.flip
            && self.config.interface == reprisekit::interface::Interface::Glass
    }

    /// Whether `w` draws as a card (inactive, no shadow): a stack is up or
    /// flying back on an output its frame meets.
    pub(crate) fn carded(&self, w: WindowId) -> bool {
        let sw = &self.switcher;
        let Some(frame) = self.win(w).filter(|_| sw.cards).map(|x| x.frame_rect()) else {
            return false;
        };
        (sw.stack.iter().chain(&sw.leaving)).any(|st| frame.intersects(st.area))
    }

    fn flip_entries(&self) -> Vec<Entry> {
        let mut entries = self.switch_entries();
        if !entries.contains(&Entry::Desktop) {
            entries.push(Entry::Desktop);
        }
        entries
    }

    /// Open the 3D switcher on the output under the pointer, on its front
    /// entry; no-op with no window or dialog. It takes the keys and pointer
    /// now and shows after `OPEN_DELAY_MS` (`flip_reveal`); releasing `mods`
    /// commits, or before it shows, closes it unseen. A stack still flying
    /// back ends first.
    pub fn open_flip(&mut self, mods: u32) {
        let entries = self.flip_entries();
        if entries.iter().all(|&e| e == Entry::Desktop) {
            return;
        }
        let p = self.pointer_location();
        let Some(output) = self.output_at(p.x, p.y) else {
            return;
        };
        if let Some(st) = self.switcher.leaving.take() {
            self.destroy_flip(st);
        }
        // The stack takes over: a drag or held button under way ends
        // without its release.
        self.cancel_interaction();
        let area = self.layout_box(Some(&output));
        let tree = self.scene.tree_create(self.layers.overlay);
        self.scene.set_data(tree, Some(SceneOwner::Switcher));
        crate::glass::leave_out(&mut self.scene, tree);
        self.scene.set_position(tree, area.x, area.y);
        let dim = self
            .scene
            .rect_create(tree, (area.width, area.height), [0.0, 0.0, 0.0, DIM]);
        self.scene.set_opacity(dim, 0.0);
        let draws = Rc::new(RefCell::new(Vec::new()));
        let node = self.scene.custom_create(
            tree,
            Box::new(CardsNode {
                ctx: self.glass.clone(),
                draws: draws.clone(),
                extent: Size::from((area.width, area.height)),
            }),
        );
        self.scene.set_input(node, Input::Passthrough);
        let sizes = entries.iter().map(|&e| self.entry_size(e, area)).collect();
        let flats = self.flip_flats(&entries, area);
        let pics = entries.iter().map(|_| Pic::new()).collect();
        let reveal = self.handle.insert_source(
            Timer::from_duration(Duration::from_millis(OPEN_DELAY_MS)),
            |_, _, state| {
                state.flip_reveal();
                TimeoutAction::Drop
            },
        );
        let reveal = match reveal {
            Ok(t) => Some(t),
            Err(e) => {
                log::warn!("3D switcher: no open timer: {}", e.error);
                return;
            }
        };
        self.switcher.stack = Some(Stack {
            entries,
            selected: 0,
            mods,
            output,
            area,
            tree,
            dim,
            sizes,
            flats,
            top: 0,
            pics,
            roll: Tween::at(0.0),
            out: Tween::at(0.0),
            shade: Tween::at(0.0),
            reveal,
            pick: false,
            cards: Vec::new(),
            draws,
            hidden: Vec::new(),
            stepping: false,
            wheel: 0,
            desk_drawn: None,
            desk_timer: None,
        });
        crate::taskpreview::hide(self);
        crate::snappickerview::hide(self);
        self.taskbar_hover(None, TaskbarHit::Nothing);
        // The keys stay with the switcher, so an open X11 menu closes here.
        crate::xwayland::menus::dismiss(self);
        let p = self.pointer_location();
        self.repoint(p, InputTime::now());
    }

    /// The bind is still held `OPEN_DELAY_MS` on: show the stack. The
    /// windows go off and their frames draw as cards show them, and the
    /// cards fly out as the dim fades in.
    fn flip_reveal(&mut self) {
        let Some(st) = self.switcher.stack.as_mut() else {
            return;
        };
        if st.reveal.take().is_none() {
            return;
        }
        let now = Instant::now();
        st.out = Tween::new(0.0, 1.0, now, OPEN_MS, Ease::Out);
        st.shade = Tween::new(0.0, 1.0, now, OPEN_MS, Ease::Linear);
        st.stepping = true;
        let area = st.area;
        let hidden = self.hide_for_flip(area);
        if let Some(st) = self.switcher.stack.as_mut() {
            st.hidden = hidden;
        }
        if !self.switcher.cards {
            self.switcher.cards = true;
            self.cards_changed();
        }
        self.flip_moved();
    }

    /// Turn the windows off on the stack's output, `area`: their layers,
    /// and every dialog's tree, which it returns. Other outputs keep theirs.
    fn hide_for_flip(&mut self, area: Rect) -> Vec<TreeId> {
        let area = Some(logical(area));
        for layer in self.flip_layers() {
            self.scene.set_hidden_in(layer, area);
        }
        let dialogs: Vec<TreeId> = self.dialogs.views.iter().map(|v| v.tree()).collect();
        for &t in &dialogs {
            self.scene.set_hidden_in(t, area);
        }
        dialogs
    }

    fn flip_layers(&self) -> [TreeId; 3] {
        let l = &self.layers;
        [l.windows, l.on_top, l.fullscreen]
    }

    fn show_after_flip(&mut self, hidden: &[TreeId]) {
        for layer in self.flip_layers() {
            self.scene.set_hidden_in(layer, None);
        }
        for &t in hidden {
            self.scene.set_hidden_in(t, None);
        }
        self.redraw_all();
    }

    /// A dialog shown while the stack is up or flying back stays under
    /// it: its tree goes off the stack's output until the cards land.
    pub(crate) fn flip_hide_dialog(&mut self, tree: TreeId) {
        let sw = &mut self.switcher;
        let open = sw.stack.as_mut().filter(|st| st.revealed());
        let Some(st) = open.or(sw.leaving.as_mut()) else {
            return;
        };
        st.hidden.push(tree);
        let area = Some(logical(st.area));
        self.scene.set_hidden_in(tree, area);
    }

    /// Where each of `entries` lies on the desktop, relative to `area`: a
    /// shown window's frame, a dialog; None for a minimized window and the
    /// desktop.
    fn flip_flats(&self, entries: &[Entry], area: Rect) -> Vec<Option<Rect>> {
        let local = |r: Rect| Rect::new(r.x - area.x, r.y - area.y, r.width, r.height);
        (entries.iter())
            .map(|&e| match e {
                Entry::Window(w) => self
                    .win(w)
                    .filter(|x| x.mapped && !x.hidden())
                    .map(|x| local(x.frame_rect())),
                Entry::Dialog(id) => (self.dialogs.views.iter())
                    .find(|v| v.id == id)
                    .map(|v| local(v.rect())),
                Entry::Desktop => None,
            })
            .collect()
    }

    /// The size `e`'s card pictures, logical: the window's frame, the
    /// dialog, or `area` for the desktop; at least 1x1.
    fn entry_size(&self, e: Entry, area: Rect) -> (i32, i32) {
        let (w, h) = match e {
            Entry::Window(w) => self
                .win(w)
                .map_or((1, 1), |x| (x.geom.total.0, x.geom.total.1)),
            Entry::Dialog(id) => (self.dialogs.views.iter())
                .find(|v| v.id == id)
                .map_or((1, 1), |v| v.layout.size),
            Entry::Desktop => (area.width, area.height),
        };
        (w.max(1), h.max(1))
    }

    /// Step the stack's selection, wrapping; `backwards` takes the previous
    /// entry. The stack rolls from where it is drawn, after the steps still
    /// rolling; before it shows, it stands there at once.
    pub fn flip_step(&mut self, backwards: bool) {
        let Some(st) = self.switcher.stack.as_mut().filter(|st| !st.pick) else {
            return;
        };
        st.selected = switcher::step(st.selected, st.entries.len(), backwards);
        let to = st.roll.target() + if backwards { -1.0 } else { 1.0 };
        if st.revealed() {
            st.roll.queue(to, Instant::now(), ROLL_MS, QUEUE_MS);
        } else {
            st.roll = Tween::at(to);
        }
        self.flip_moved();
    }

    /// A click on entry `k`'s card: the front one commits at once; a
    /// card further back rolls to the front over `PICK_MS` and commits
    /// there (`flip_frame`).
    pub fn flip_pick(&mut self, k: usize) {
        let Some(st) = self.switcher.stack.as_mut() else {
            return;
        };
        let n = st.entries.len();
        if st.pick || !st.revealed() || k >= n {
            return;
        }
        let ahead = (k + n - st.selected) % n;
        if ahead == 0 {
            self.switcher_commit();
            return;
        }
        st.selected = k;
        st.pick = true;
        let to = st.roll.target() + ahead as f64;
        st.roll.retarget(to, Instant::now(), PICK_MS);
        self.flip_moved();
    }

    /// The wheel over the open stack: each notch up brings the entry
    /// before to the front, each one down the next.
    pub fn flip_wheel(&mut self, discrete: i32, continuous: f64) {
        let Some(st) = self.switcher.stack.as_mut() else {
            return;
        };
        let (steps, residual) = crate::pointers::wheel_steps(discrete, continuous, st.wheel);
        st.wheel = residual;
        for _ in 0..steps.unsigned_abs() {
            self.flip_step(steps > 0);
        }
    }

    pub fn flip_hit(&self, lx: f64, ly: f64) -> Option<usize> {
        let st = self.switcher.stack.as_ref()?;
        let (x, y) = (lx - f64::from(st.area.x), ly - f64::from(st.area.y));
        flip::hit(&st.cards, x, y)
    }

    /// The open stack's selection or list changed, or it began to move:
    /// draw it where it is now, and ask its output for the frames that
    /// move it on.
    pub(crate) fn flip_moved(&mut self) {
        let Some(output) = self.switcher.stack.as_ref().map(|st| st.output.clone()) else {
            return;
        };
        self.flip_draw(false, Instant::now());
        crate::backend::schedule(self, &output);
        crate::access::touch(self);
    }

    /// `output` is about to render a frame: put the open or leaving
    /// stack on it where it is due now and ask for the next frame while it
    /// moves. A leaving stack that has landed goes, and the windows show
    /// again in this frame, under where its cards were.
    pub fn flip_frame(&mut self, output: &Output) {
        let now = Instant::now();
        for leaving in [false, true] {
            let sw = &self.switcher;
            let st = if leaving { &sw.leaving } else { &sw.stack };
            let Some(st) = st.as_ref().filter(|st| &st.output == output) else {
                continue;
            };
            if !leaving && st.pick && st.roll.settled(now) {
                self.switcher_commit();
                continue;
            }
            let moving = st.moving(now);
            if leaving && !moving {
                if let Some(st) = self.switcher.leaving.take() {
                    self.destroy_flip(st);
                }
                let p = self.pointer_location();
                self.repoint(p, InputTime::now());
                continue;
            }
            if moving || st.stepping {
                self.flip_draw(leaving, now);
                if !moving && !leaving {
                    crate::access::touch(self);
                }
            }
            self.flip_desktop_tick(leaving);
            if moving {
                crate::backend::schedule(self, output);
            }
        }
    }

    /// Lay the open stack, or the leaving one, out for `now` and update
    /// its node and the dim: the drawn entries get their pictures, the
    /// rest drop their live ones.
    fn flip_draw(&mut self, leaving: bool, now: Instant) {
        let sw = &mut self.switcher;
        let Some(st) = (if leaving {
            &mut sw.leaving
        } else {
            &mut sw.stack
        })
        .as_mut() else {
            return;
        };
        if !st.revealed() {
            return;
        }
        let local = Rect::new(0, 0, st.area.width, st.area.height);
        let out = st.out.value(now);
        st.stepping = st.moving(now);
        let cards = flip::fly(
            local,
            &st.sizes,
            &st.flats,
            st.top,
            st.roll.value(now),
            out,
            &GEOMETRY,
        );
        let scale = st.output.current_scale().fractional_scale();
        let (entries, sizes, dim) = (st.entries.clone(), st.sizes.clone(), st.dim);
        let shade = st.shade.value(now) as f32;
        self.scene.set_opacity(dim, shade);
        for (k, (&e, &size)) in entries.iter().zip(&sizes).enumerate() {
            let drawn = cards.iter().any(|c| c.entry == k);
            let px = (
                ((f64::from(size.0) * scale).round() as i32).max(1),
                ((f64::from(size.1) * scale).round() as i32).max(1),
            );
            self.place_flip_picture(leaving, k, e, drawn, px);
        }
        let sw = &mut self.switcher;
        let Some(st) = (if leaving {
            &mut sw.leaving
        } else {
            &mut sw.stack
        })
        .as_mut() else {
            return;
        };
        *st.draws.borrow_mut() = (cards.iter().rev())
            .map(|c| {
                let pic = &st.pics[c.entry];
                Draw {
                    card: pic.card.clone(),
                    slot: pic.slot.clone(),
                    to_screen: c.to_screen,
                    size: c.size,
                    alpha: c.alpha,
                }
            })
            .collect();
        st.cards = cards;
    }

    /// Give entry `k` (`e`) its picture at `px` device pixels when its
    /// card is `drawn`, or take a live one down when not: a window or
    /// the desktop live, a dialog as a still made once.
    fn place_flip_picture(
        &mut self,
        leaving: bool,
        k: usize,
        e: Entry,
        drawn: bool,
        px: (i32, i32),
    ) {
        let Some(pic) = self.flip_pic(leaving, k) else {
            return;
        };
        if !drawn {
            if let Some(id) = pic.thumb.take() {
                *pic.slot.borrow_mut() = None;
                self.thumb_drop(id);
            } else if pic.desk.take().is_some() {
                *pic.slot.borrow_mut() = None;
            }
            return;
        }
        if pic.thumb.is_some() || pic.still || pic.desk.is_some() {
            return;
        }
        let slot = pic.slot.clone();
        match e {
            Entry::Window(w) => {
                let thumb = self.whole_thumb_create(w, px, slot);
                if let Some(pic) = self.flip_pic(leaving, k) {
                    pic.thumb = thumb;
                }
            }
            Entry::Dialog(id) => {
                let Some((tree, rect)) = (self.dialogs.views.iter())
                    .find(|v| v.id == id)
                    .map(|v| (v.tree(), v.rect()))
                else {
                    return;
                };
                let scale = f64::from(px.0) / f64::from(rect.width.max(1));
                *slot.borrow_mut() = self.flip_still(tree, rect, scale);
                if let Some(pic) = self.flip_pic(leaving, k) {
                    pic.still = true;
                }
            }
            Entry::Desktop => {
                let Some((area, scale)) = self.flip_desktop_view(leaving) else {
                    return;
                };
                let Some(target) = self.tree_target(area, scale, true) else {
                    return;
                };
                *slot.borrow_mut() = Some(target.texture().clone());
                if let Some(pic) = self.flip_pic(leaving, k) {
                    pic.desk = Some(target);
                }
                self.flip_desktop_draw(leaving);
            }
        }
    }

    fn flip_still(
        &mut self,
        tree: TreeId,
        rect: Rect,
        scale: f64,
    ) -> Option<smithay::backend::renderer::gles::GlesTexture> {
        let texture = self.tree_still(tree, rect, scale, false)?;
        let renderer = &mut self.backend.as_mut()?.renderer;
        if let Err(e) = reprisefx::mipmaps(renderer, &texture) {
            log::warn!("3D switcher: no mipmaps: {e}");
        }
        Some(texture)
    }

    /// What the desktop's picture covers for the open stack, or the
    /// leaving one: the output's layout box, and its scale.
    fn flip_desktop_view(&self, leaving: bool) -> Option<(Rect, f64)> {
        let sw = &self.switcher;
        let st = if leaving { &sw.leaving } else { &sw.stack };
        let st = st.as_ref()?;
        Some((st.area, st.output.current_scale().fractional_scale()))
    }

    /// A frame of the stack's output: the desktop's live picture of the
    /// open stack, or the leaving one, draws now when its last draw is
    /// `[previews] fps` behind, else a timer asks for the frame that
    /// draws it then (what changed meanwhile may bring no frame).
    fn flip_desktop_tick(&mut self, leaving: bool) {
        let period = self.preview_period();
        let now = Instant::now();
        let sw = &self.switcher;
        let st = if leaving { &sw.leaving } else { &sw.stack };
        let Some(st) = st.as_ref() else {
            return;
        };
        if st.desk_timer.is_some() || !st.pics.iter().any(|p| p.desk.is_some()) {
            return;
        }
        let wait = crate::livepreview::preview_wait(st.desk_drawn, now, period);
        if wait.is_zero() {
            self.flip_desktop_draw(leaving);
            return;
        }
        let output = st.output.clone();
        let timer = self
            .handle
            .insert_source(Timer::from_duration(wait), move |_, _, state| {
                let sw = &mut state.switcher;
                for st in [sw.stack.as_mut(), sw.leaving.as_mut()]
                    .into_iter()
                    .flatten()
                {
                    if st.output == output {
                        st.desk_timer = None;
                    }
                }
                crate::backend::schedule(state, &output);
                TimeoutAction::Drop
            });
        let timer = match timer {
            Ok(t) => t,
            Err(e) => {
                log::warn!("3D switcher: no desktop timer: {}", e.error);
                return;
            }
        };
        let sw = &mut self.switcher;
        if let Some(st) = (if leaving {
            &mut sw.leaving
        } else {
            &mut sw.stack
        })
        .as_mut()
        {
            st.desk_timer = Some(timer);
        }
    }

    /// Bring the desktop's live picture of the open stack, or the leaving
    /// one, up to date, while its card is drawn: draw what changed in the
    /// background layer since its last draw, and its mipmaps after.
    fn flip_desktop_draw(&mut self, leaving: bool) {
        let sw = &self.switcher;
        let st = if leaving { &sw.leaving } else { &sw.stack };
        let Some(k) =
            (st.as_ref()).and_then(|st| st.entries.iter().position(|&e| e == Entry::Desktop))
        else {
            return;
        };
        let Some((area, scale)) = self.flip_desktop_view(leaving) else {
            return;
        };
        let Some(mut target) = self.flip_pic(leaving, k).and_then(|p| p.desk.take()) else {
            return;
        };
        let tree = self.layers.background;
        if self.tree_render(tree, area, scale, &mut target) == Some(true)
            && let Some(b) = self.backend.as_mut()
            && let Err(e) = reprisefx::mipmaps(&mut b.renderer, target.texture())
        {
            log::warn!("3D switcher: no mipmaps: {e}");
        }
        if let Some(pic) = self.flip_pic(leaving, k) {
            pic.desk = Some(target);
        }
        let sw = &mut self.switcher;
        if let Some(st) = (if leaving {
            &mut sw.leaving
        } else {
            &mut sw.stack
        })
        .as_mut()
        {
            st.desk_drawn = Some(Instant::now());
        }
    }

    fn flip_pic(&mut self, leaving: bool, k: usize) -> Option<&mut Pic> {
        let sw = &mut self.switcher;
        let st = if leaving {
            &mut sw.leaving
        } else {
            &mut sw.stack
        };
        st.as_mut()?.pics.get_mut(k)
    }

    /// Close the open stack: its cards fly back to the desktop as the dim
    /// fades. `picked` is the entry a commit switched to, which lands on top;
    /// None for a cancel.
    pub(crate) fn flip_leave(&mut self, mut st: Stack, picked: Option<usize>) {
        st.sizes = (st.entries.iter())
            .map(|&e| self.entry_size(e, st.area))
            .collect();
        st.flats = self.flip_flats(&st.entries, st.area);
        if let Some(k) = picked {
            st.top = k;
        }
        let now = Instant::now();
        st.out = Tween::new(st.out.value(now), 0.0, now, CLOSE_MS, Ease::InOut);
        st.shade.retarget(0.0, now, CLOSE_MS);
        st.pick = false;
        self.scene.set_input(st.dim, Input::Passthrough);
        let output = st.output.clone();
        // The one it replaces goes with this one in place, so the frames
        // stay drawn for the cards.
        if let Some(old) = self.switcher.leaving.replace(st) {
            self.destroy_flip(old);
        }
        crate::backend::schedule(self, &output);
        crate::access::touch(self);
    }

    /// Take a stack down at once: its timer, live pictures, then its
    /// tree, and show the windows again; with no other stack left, the
    /// frames draw for the desktop again.
    pub(crate) fn destroy_flip(&mut self, st: Stack) {
        for t in [st.reveal, st.desk_timer].into_iter().flatten() {
            self.handle.remove(t);
        }
        for id in st.pics.iter().filter_map(|p| p.thumb) {
            self.thumb_drop(id);
        }
        self.scene.destroy(st.tree);
        self.show_after_flip(&st.hidden);
        let sw = &self.switcher;
        if sw.cards && sw.stack.is_none() && sw.leaving.is_none() {
            self.switcher.cards = false;
            self.cards_changed();
        }
        crate::access::touch(self);
    }

    /// Drop `entry` from the open stack and the leaving one
    /// (`switcher_forget`): its card goes and the cards behind it move up
    /// into its place. False when it was not one of the open stack's.
    pub(crate) fn flip_forget(&mut self, entry: Entry) -> bool {
        let now = Instant::now();
        let leaving = self.switcher.leaving.as_mut();
        if let Some(pic) = leaving.and_then(|st| forget_entry(st, entry, now)) {
            if let Some(id) = pic.thumb {
                self.thumb_drop(id);
            }
            self.flip_draw(true, now);
            if let Some(o) = self.switcher.leaving.as_ref().map(|st| st.output.clone()) {
                crate::backend::schedule(self, &o);
            }
        }
        let open = self.switcher.stack.as_mut();
        let Some(pic) = open.and_then(|st| forget_entry(st, entry, now)) else {
            return false;
        };
        if let Some(id) = pic.thumb {
            self.thumb_drop(id);
        }
        true
    }

    /// A window or dialog opened (`map_window`, `open_dialog`): one the open
    /// stack lists but does not hold joins it before the desktop
    /// (`flip::insert_roll`), shown as its card.
    pub(crate) fn flip_join(&mut self) {
        let Some(st) = self.switcher.stack.as_ref() else {
            return;
        };
        let area = st.area;
        let fresh: Vec<Entry> = (self.flip_entries().into_iter())
            .filter(|e| !st.entries.contains(e))
            .collect();
        if fresh.is_empty() {
            return;
        }
        let now = Instant::now();
        for e in fresh {
            let size = self.entry_size(e, area);
            let flat = self.flip_flats(&[e], area)[0];
            let Some(st) = self.switcher.stack.as_mut() else {
                return;
            };
            join_entry(st, e, size, flat, now);
        }
        self.flip_moved();
    }

    /// `w`'s frame changed size or place (`relayout`): its card in the
    /// open or leaving stack takes the new size, with a new picture at
    /// it, and flies from or to where the window now lies.
    pub(crate) fn flip_window_changed(&mut self, w: WindowId) {
        let e = Entry::Window(w);
        for leaving in [false, true] {
            let sw = &self.switcher;
            let st = if leaving { &sw.leaving } else { &sw.stack };
            let Some((k, area)) = st
                .as_ref()
                .and_then(|st| Some((st.entries.iter().position(|&x| x == e)?, st.area)))
            else {
                continue;
            };
            let size = self.entry_size(e, area);
            let flat = self.flip_flats(&[e], area)[0];
            let sw = &mut self.switcher;
            let Some(st) = (if leaving {
                &mut sw.leaving
            } else {
                &mut sw.stack
            })
            .as_mut() else {
                continue;
            };
            if st.sizes[k] == size && st.flats[k] == flat {
                continue;
            }
            st.flats[k] = flat;
            let mut drop = None;
            if st.sizes[k] != size {
                st.sizes[k] = size;
                drop = st.pics[k].thumb.take();
                *st.pics[k].slot.borrow_mut() = None;
            }
            let output = st.output.clone();
            if let Some(id) = drop {
                self.thumb_drop(id);
            }
            self.flip_draw(leaving, Instant::now());
            crate::backend::schedule(self, &output);
        }
    }

    pub fn flip_access(&self) -> Option<reprisekit::access::Tree> {
        use reprisekit::access::{ROOT, accesskit::Role};
        let st = self.switcher.stack.as_ref()?;
        let titles: Vec<String> = st.entries.iter().map(|&e| self.entry_title(e)).collect();
        let mut tree = crate::access::root(Role::Window, "Task Switcher", st.area);
        let local = Rect::new(0, 0, st.area.width, st.area.height);
        let focus = flip::access(&titles, st.selected, &st.cards, local, &mut tree, ROOT);
        tree.set_focus(focus);
        Some(tree)
    }
}

fn logical(r: Rect) -> Rectangle<i32, Logical> {
    Rectangle::new((r.x, r.y).into(), (r.width, r.height).into())
}

/// Put `entry` (`size`, lying at `flat`) into `st` at `now`, before the
/// desktop: the position, the selection and the top entry move so every
/// card keeps its place (`flip::insert_roll`).
fn join_entry(st: &mut Stack, entry: Entry, size: (i32, i32), flat: Option<Rect>, now: Instant) {
    let n = st.entries.len();
    let i = (st.entries.iter())
        .position(|&e| e == Entry::Desktop)
        .unwrap_or(n);
    let (at, to) = flip::insert_roll(st.roll.value(now), st.roll.target(), n, i);
    st.roll = Tween::at(at);
    if to != at {
        st.roll.retarget(to, now, ROLL_MS);
    }
    if st.selected >= i {
        st.selected += 1;
    }
    if st.top >= i {
        st.top += 1;
    }
    st.entries.insert(i, entry);
    st.sizes.insert(i, size);
    st.flats.insert(i, flat);
    st.pics.insert(i, Pic::new());
}

/// Take `entry` out of `st` at `now`: the position and selection move so
/// the cards in front stay and the ones behind move up one
/// (`flip::forget_roll`). Returns its picture, for its live thumb to drop;
/// None when it was not one of `st`'s.
fn forget_entry(st: &mut Stack, entry: Entry, now: Instant) -> Option<Pic> {
    let k = st.entries.iter().position(|&e| e == entry)?;
    let n = st.entries.len();
    if st.pick && st.selected == k {
        st.pick = false;
    }
    st.selected = flip::forget(st.selected as f64, n, k).round() as usize;
    let (at, to) = flip::forget_roll(st.roll.value(now), st.roll.target(), n, k);
    st.roll = Tween::at(at);
    if to != at {
        st.roll.retarget(to, now, ROLL_MS);
    }
    st.entries.remove(k);
    st.sizes.remove(k);
    st.flats.remove(k);
    st.top = match st.top {
        t if t > k => t - 1,
        t if t == k => 0,
        t => t,
    };
    Some(st.pics.remove(k))
}
