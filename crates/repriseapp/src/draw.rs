//! Drawing a window into its shm buffers at the output scale, and the surface and output events.

use crate::look::CAPTION_FIT;
use crate::runloop::NoEvents;
use crate::sizing::{at_pick, draw_scale};
use crate::{Event, Host, MAIN, Scaling, Shell, Slot, WinId, filedlg};
use reprisekit::controls::form;
use reprisekit::controls::menu::MenuCtx;
use reprisekit::raster::{Canvas, Rect};
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, FrameCallbackData},
    dispatch2::Dispatch2,
    output::{OutputHandler, OutputState},
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use std::time::Duration;
use wayland_client::{
    Connection, QueueHandle,
    protocol::{wl_output, wl_shm, wl_surface},
};

impl Host {
    pub(crate) fn scaling(&self, surface: &wl_surface::WlSurface) -> Scaling {
        Scaling {
            fraction_120: None,
            buffer_scale: self.output_scale,
            drawn: 0,
            viewport: self
                .viewporter
                .as_ref()
                .map(|v| v.get_viewport(surface, &self.qh, NoEvents)),
            fraction: self
                .fractional
                .as_ref()
                .map(|f| f.get_fractional_scale(surface, &self.qh, SurfaceOf(surface.clone()))),
        }
    }

    fn rescale(
        &mut self,
        surface: &wl_surface::WlSurface,
        fraction_120: Option<u32>,
        buffer_scale: Option<i32>,
    ) {
        let Some(id) = self.slot_of(surface) else {
            return;
        };
        let Some(s) = self.slot_mut(id) else {
            return;
        };
        if let Some(n) = fraction_120 {
            s.scaling.fraction_120 = Some(n);
        }
        if let Some(n) = buffer_scale {
            s.scaling.buffer_scale = n.max(1);
        }
        if draw_scale(s.scaling.fraction_120, s.scaling.buffer_scale, s.size) != s.scaling.drawn {
            s.dirty = true;
        }
    }

    /// Keeps [`Ctx::output_size`](crate::Ctx::output_size) on the largest output but `gone`, which
    /// the output state still lists while it is being destroyed.
    fn outputs_changed(&mut self, gone: Option<&wl_output::WlOutput>) {
        self.ctx.output_size = self
            .output_state
            .outputs()
            .filter(|o| Some(o) != gone)
            .filter_map(|o| self.output_state.info(&o)?.logical_size)
            .max_by_key(|&(w, h)| i64::from(w) * i64::from(h));
    }

    /// Set slot `i`'s blur region to its top `rows.1` rows across its `rows.0` width, for the
    /// commit that follows; none at 0.
    fn set_glass_top(&mut self, i: usize, rows: (i32, i32), qh: &QueueHandle<Host>) {
        let Some(effects) = &self.effects else {
            return;
        };
        let s = &mut self.slots[i];
        let had = s.glass_top.1 > 0;
        if s.glass_top == rows || (!had && rows.1 == 0) {
            return;
        }
        s.glass_top = rows;
        let surface = s.window.wl_surface();
        let effect = s
            .glass
            .get_or_insert_with(|| effects.get_background_effect(surface, qh, NoEvents));
        if rows.1 == 0 {
            effect.set_blur_region(None);
            return;
        }
        match smithay_client_toolkit::compositor::Region::new(&self.compositor) {
            Ok(region) => {
                region.add(0, 0, rows.0, rows.1);
                effect.set_blur_region(Some(region.wl_region()));
            }
            Err(e) => log::warn!("blur region: {e}"),
        }
    }

    pub(crate) fn draw(&mut self, id: WinId, qh: &QueueHandle<Host>) {
        self.draw_frame(id, qh, false);
    }

    /// Draws window `id`; a `blink_only` frame answers a caret blink and nothing else
    /// ([`Canvas::blink_only`]).
    pub(crate) fn draw_frame(&mut self, id: WinId, qh: &QueueHandle<Host>, blink_only: bool) {
        let Some(i) = self.slots.iter().position(|s| s.id == id) else {
            return;
        };
        if self.slots[i].frame_pending {
            self.slots[i].dirty = true;
            return;
        }
        let (w, h) = self.slots[i].size;
        let sc = &mut self.slots[i].scaling;
        let scale = draw_scale(sc.fraction_120, sc.buffer_scale, (w, h));
        if std::mem::replace(&mut sc.drawn, scale) != scale {
            self.slots[i].buffers = [None, None];
            self.slots[i].pool = None;
        }
        let (bw, bh) = (w * scale, h * scale);
        let k = match ready_buffer(&mut self.slots[i], &self.shm, (bw, bh)) {
            Ok(k) => k,
            Err(e) => return self.no_buffer(i, &e),
        };
        if self.shm_trouble.drawn() {
            log::info!("shm buffers are back");
        }
        self.ctx.icons.set_scale(scale);
        // Cleared: an app may leave a region alone and expect it transparent.
        let mut canvas = std::mem::replace(&mut self.slots[i].canvas, Canvas::new(0, 0));
        let fresh = (canvas.width, canvas.height, canvas.scale()) == (w as u32, h as u32, scale);
        if !fresh {
            canvas = Canvas::with_scale(w as u32, h as u32, scale);
        } else if !self.app.keeps_canvas(id) {
            canvas.pixels.fill(0);
        }
        self.ctx.size = (w, h);
        self.ctx.maximized = self.slots[i].maximized;
        self.ctx.at_pick = at_pick((w, h), self.slots[i].picked, self.slots[i].bounds);
        self.ctx.origin = self.slots[i].origin;
        // Only the active window draws as active (a dialog's default
        // button): a toplevel as the compositor
        // says, a flyout while it has the keyboard.
        self.ctx.theme.active = match self.slots[i].window {
            Shell::Toplevel(_) => self.slots[i].activated,
            Shell::Layer(_) => self.focus == Some(id),
        };
        self.ctx.theme.access_keys = self.slots[i].cues.keys;
        let preedit = self
            .text_input
            .as_ref()
            .filter(|t| t.entered == Some(id))
            .and_then(|t| t.preedit().cloned());
        canvas.set_preedit(preedit);
        canvas.set_blink_only(blink_only);
        filedlg::follow_scale(&mut *self.app, id, scale, &mut self.ctx);
        self.app.follow_scale(id, scale, &mut self.ctx);
        let paste = self.edit_menu.is_pending(id) && self.clipboard_text();
        let within = self.menu_within((w, h));
        let app = &mut self.app;
        let menu = &mut self.edit_menu;
        let last_tip = &mut self.slots[i].tip;
        let spot = self.ctx.at_scale(scale, |ctx| {
            app.draw(id, &mut canvas, ctx);
            // A form's label tip goes over everything; a kept canvas would keep its pixels.
            if app.keeps_canvas(id) {
                canvas.take_tip();
            } else {
                let tip = form::draw_tip(&mut canvas, &ctx.text, &ctx.forms());
                for r in tip.iter().chain(last_tip.iter()) {
                    canvas.widen_damage(*r);
                }
                *last_tip = tip;
            }
            let spot = canvas.take_spot();
            let cx = MenuCtx {
                text: &ctx.text,
                theme: &ctx.theme,
                px: ctx.theme.fonts.size_px,
                within,
            };
            menu.drawn(id, spot.as_ref(), paste, &cx);
            spot
        });
        self.slots[i].spot = spot.clone();
        if spot.is_some() {
            self.arm_blink();
        }
        // The window's draw follows its model; so do its menu's popups.
        self.menu_changed(id);
        let glass_top = canvas.take_glass_top();
        let glass_top = match reprisekit::look::client_glass(&self.ctx.theme, true) {
            Some(_) => glass_top.clamp(0, h),
            None => 0,
        };
        self.ctx.theme.active = true;
        self.ctx.theme.access_keys = false;
        self.set_glass_top(i, (w, glass_top), qh);
        let damage = canvas.take_damage();
        canvas.set_blink_only(false);
        if let Some(t) = self.text_input.as_mut() {
            t.sync(id, spot);
        }
        // After the draw: the tree reads layout the draw cached.
        let mapped = self.slots[i].mapped;
        self.slots[i].access.state.after_draw(mapped);
        self.publish(id);
        let slot = &mut self.slots[i];
        let pool = slot.pool.as_mut().expect("ready_buffer made the pool");
        let buffer = slot.buffers[k]
            .as_ref()
            .expect("ready_buffer made the buffer");
        let target = pool.canvas(buffer).expect("a released buffer");
        copy_pixels(target, &canvas.pixels);
        let s = &mut self.slots[i];
        s.attached = k;
        s.canvas = canvas;
        let surface = s.window.wl_surface();
        match &s.scaling.viewport {
            Some(v) => v.set_destination(w, h),
            None => surface.set_buffer_scale(scale),
        }
        let partial = blink_only || self.app.keeps_canvas(id);
        for (x, y, w, h) in surface_damage(damage, fresh, partial, scale, (bw, bh)) {
            surface.damage_buffer(x, y, w, h);
        }
        surface.frame(qh, FrameCallbackData(surface.clone()));
        s.frame_pending = true;
        s.buffers[k]
            .as_ref()
            .expect("buffer")
            .attach_to(surface)
            .expect("attach");
        s.window.commit();
        if !std::mem::replace(&mut s.mapped, true) {
            self.sync_children();
        }
    }
}

impl Host {
    /// No shm buffer for slot `i`'s frame: it stays dirty and a timer tries again shortly. The
    /// first failure of a run is logged; a run that goes on is told to the program
    /// ([`Extended::draw_failed`](crate::Extended::draw_failed)).
    fn no_buffer(&mut self, i: usize, e: &str) {
        self.slots[i].dirty = true;
        let (log, tell) = self.shm_trouble.failed();
        if log {
            log::warn!("no shm buffer for a frame, trying again: {e}");
        }
        if !self.shm_retry {
            let timer = Timer::from_duration(SHM_RETRY);
            let armed = self
                .loop_handle
                .insert_source(timer, |_, _, host: &mut Host| {
                    host.shm_retry = false;
                    TimeoutAction::Drop
                });
            self.shm_retry = armed.is_ok();
        }
        if tell {
            let reply = self.app.draw_failed(e);
            self.apply(MAIN, reply);
        }
    }
}

/// The wait before a frame that found no shm buffer tries again.
const SHM_RETRY: Duration = Duration::from_millis(100);

/// Failed tries in a row after which the program hears that its windows cannot draw: 5 s.
pub(crate) const SHM_TELL_AFTER: u32 = 50;

/// A run of frames that found no shm buffer (`/dev/shm` full, out of file descriptors).
#[derive(Debug, Default)]
pub(crate) struct ShmTrouble {
    failures: u32,
}

impl ShmTrouble {
    /// One more failed try: whether to log it (the first of a run) and whether to tell the
    /// program (once, at [`SHM_TELL_AFTER`]).
    pub(crate) fn failed(&mut self) -> (bool, bool) {
        self.failures = self.failures.saturating_add(1);
        (self.failures == 1, self.failures == SHM_TELL_AFTER)
    }

    /// A frame got its buffer; whether that ended a run.
    pub(crate) fn drawn(&mut self) -> bool {
        std::mem::take(&mut self.failures) > 0
    }
}

/// The buffer `slot`'s next frame of `size` buffer px goes into, its pool and the buffer made
/// when missing.
fn ready_buffer(slot: &mut Slot, shm: &Shm, (bw, bh): (i32, i32)) -> Result<usize, String> {
    let stride = bw * 4;
    if slot.pool.is_none() {
        // SCTK rounds a slot up to 64 bytes; a pool a byte short of
        // that doubles itself on the first allocation.
        let len = ((bh * stride) as usize + 63) & !63;
        let pool = SlotPool::new(len.max(64), shm).map_err(|e| format!("shm pool: {e}"))?;
        slot.pool = Some(pool);
    }
    let pool = slot.pool.as_mut().expect("made above");
    let free = slot
        .buffers
        .each_ref()
        .map(|b| b.as_ref().map(|b| b.canvas(pool).is_some()));
    let (k, make) = next_buffer(free, slot.attached);
    if make {
        let (buffer, _) = pool
            .create_buffer(bw, bh, stride, wl_shm::Format::Argb8888)
            .map_err(|e| format!("shm buffer: {e}"))?;
        slot.buffers[k] = Some(buffer);
    }
    Ok(k)
}

/// Which of a slot's two buffers the next frame goes into, and whether to
/// make it. `free[k]` is `None` for a buffer not made yet, else whether the
/// compositor released it. Reuse a released one, else make a missing one,
/// else replace the one not attached last.
pub(crate) fn next_buffer(free: [Option<bool>; 2], attached: usize) -> (usize, bool) {
    if let Some(k) = free.iter().position(|f| *f == Some(true)) {
        (k, false)
    } else if let Some(k) = free.iter().position(Option::is_none) {
        (k, true)
    } else {
        (1 - attached, true)
    }
}

/// Reported logical rects as buffer-px `(x, y, width, height)` at `scale`.
pub(crate) fn buffer_damage(rects: &[Rect], scale: i32) -> Vec<(i32, i32, i32, i32)> {
    rects
        .iter()
        .map(|r| (r.x * scale, r.y * scale, r.width * scale, r.height * scale))
        .collect()
}

/// The buffer-px rects to damage: the reported ones only for a `partial` frame (the app keeps its
/// canvas, or the frame only flipped the caret) on a `fresh` canvas, else the whole `buffer`.
pub(crate) fn surface_damage(
    damage: Option<Vec<Rect>>,
    fresh: bool,
    partial: bool,
    scale: i32,
    buffer: (i32, i32),
) -> Vec<(i32, i32, i32, i32)> {
    match (fresh && partial).then_some(damage).flatten() {
        Some(rects) => buffer_damage(&rects, scale),
        None => vec![(0, 0, buffer.0, buffer.1)],
    }
}

pub(crate) fn copy_pixels(target: &mut [u8], pixels: &[u32]) {
    let n = pixels.len().min(target.len() / 4);
    #[cfg(target_endian = "little")]
    {
        // SAFETY: `u32` has no padding and any bit pattern is a valid
        // byte; the slice covers exactly `n` words of `pixels`.
        let bytes = unsafe { std::slice::from_raw_parts(pixels.as_ptr() as *const u8, n * 4) };
        target[..n * 4].copy_from_slice(bytes);
    }
    #[cfg(not(target_endian = "little"))]
    for (dst, px) in target.as_chunks_mut::<4>().0.iter_mut().zip(&pixels[..n]) {
        *dst = px.to_le_bytes();
    }
}

impl CompositorHandler for Host {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        factor: i32,
    ) {
        if !self.rescale_drag_icon(surface, factor) {
            self.rescale(surface, None, Some(factor));
        }
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        let Some(id) = self.slot_of(surface) else {
            return;
        };
        let Some(s) = self.slot_mut(id) else {
            return;
        };
        s.frame_pending = false;
        if s.dirty && s.configured {
            s.dirty = false;
            self.draw(id, qh);
        }
    }
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        output: &wl_output::WlOutput,
    ) {
        let info = self.output_state.info(output);
        if let Some(size) = info.as_ref().and_then(|i| i.logical_size)
            && self
                .slots
                .iter()
                .any(|s| s.id == MAIN && s.window.wl_surface() == surface)
        {
            CAPTION_FIT.with_borrow_mut(|(_, w)| *w = Some(size.0));
            self.send(MAIN, Event::Output(size));
        }
        let Some((x, y)) = info.and_then(|i| i.logical_position) else {
            return;
        };
        if let Some(s) = self
            .slots
            .iter_mut()
            .find(|s| s.window.toplevel().is_none() && s.window.wl_surface() == surface)
            && s.origin != (x, y)
        {
            s.origin = (x, y);
            s.dirty = true;
        }
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Host {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.outputs_changed(None);
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.outputs_changed(None);
    }
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, o: wl_output::WlOutput) {
        self.outputs_changed(Some(&o));
    }
}

impl ShmHandler for Host {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

struct SurfaceOf(wl_surface::WlSurface);

impl Dispatch2<wp_fractional_scale_v1::WpFractionalScaleV1, Host> for SurfaceOf {
    fn event(
        &self,
        host: &mut Host,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            host.rescale(&self.0, Some(scale), None);
        }
    }
}
