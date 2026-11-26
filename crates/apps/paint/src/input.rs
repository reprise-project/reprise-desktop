//! Keys and the pointer in the main window.

use super::*;
use crate::doc::{Effect, Ink, Mods};
use crate::view::{self, Options, options_of};
use repriseapp::{BTN_LEFT, BTN_RIGHT, KeyInput, PointerInput};
use reprisekit::classic::widgets::LINE_H;
use reprisekit::controls::menubar::{Action, Key};
use reprisekit::controls::multiedit::MultiKey;
use reprisekit::controls::palette;
use reprisekit::controls::statusbar;
use reprisekit::controls::toolbox::{self, Hit};
use reprisekit::pointer::Shape;
use reprisekit::textedit::{EditKey, keysym};
use std::time::Instant;

const SHIFT_L: u32 = 0xffe1;
const SHIFT_R: u32 = 0xffe2;

impl Paint {
    pub(crate) fn main_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        match ev {
            Event::Key(k) => self.key(k, ctx),
            Event::KeyUp(k) => self.key_up(k, ctx),
            Event::Pointer(p) => self.pointer(p, ctx),
            Event::Pasted(text) => {
                if self.doc.text_box.is_some() && self.text.paste(&text) {
                    return Reply::redraw();
                }
                Reply::none()
            }
            Event::PastedImage(bytes) => self.paste_image(&bytes, ctx),
            Event::Tick => self.tick(ctx),
            Event::CloseRequest => self.confirm(Then::Quit),
            Event::Focus(false) => {
                self.alt_arm.interrupt();
                self.bar.close();
                Reply::redraw()
            }
            Event::Resized => {
                self.hold_scroll(ctx);
                Reply::redraw()
            }
            Event::Blink | Event::Focus(_) => Reply::redraw(),
            _ => Reply::none(),
        }
    }

    fn paste_image(&mut self, bytes: &[u8], ctx: &Ctx) -> Reply {
        if bytes.is_empty() {
            return Reply::none();
        }
        match file::decode_with(&self.decoders, bytes) {
            Ok(pic) => {
                self.commit_text();
                self.pick_tool(Tool::Select);
                let l = self.lay(ctx);
                let (x, y) = self.to_picture(&l, l.view.x, l.view.y);
                self.doc.paste(pic, (x.max(0), y.max(0)));
                self.hold_scroll(ctx);
            }
            Err(e) => {
                log::info!("paste: {e}");
                self.error(tr!("paste-failed"));
            }
        }
        Reply::redraw_all()
    }

    fn tick(&mut self, ctx: &Ctx) -> Reply {
        if self.doc.spraying() {
            self.doc.tick();
            return Reply {
                redraw: true,
                tick: true,
                ..Reply::none()
            };
        }
        let now = Instant::now();
        let (h, v) = (self.hscroll.tick(now), self.vscroll.tick(now));
        if let Some(to) = h {
            self.hscroll.top = to;
        }
        if let Some(to) = v {
            self.vscroll.top = to;
        }
        self.hold_scroll(ctx);
        Reply {
            redraw: h.is_some() || v.is_some(),
            tick: self.hscroll.ticking() || self.vscroll.ticking(),
            ..Reply::none()
        }
    }

    fn key(&mut self, k: KeyInput, ctx: &Ctx) -> Reply {
        if matches!(k.sym, SHIFT_L | SHIFT_R) {
            self.shift = true;
        }
        if matches!(k.sym, keysym::ALT_L | keysym::ALT_R) {
            self.alt_arm.press_alt();
            return Reply::none();
        }
        self.alt_arm.interrupt();
        let lay = self.bar_layout(ctx);
        let px = ctx.theme.fonts.size_px;
        let bar_key = Key::from_input(k.sym, k.utf8.as_deref(), k.mods.shift, k.mods.alt);
        if let Some(bk) = bar_key {
            match self.bar.key(bk, &lay, &ctx.text, &ctx.theme, px) {
                Action::Chosen { title, row, .. } => return self.command(title, row, ctx),
                Action::Redraw => return Reply::redraw(),
                Action::Ignored | Action::Passed => {}
            }
        }
        if self.doc.text_box.is_some() {
            return self.text_key(&k);
        }
        if let Some(r) = self.accel(&k, ctx) {
            return r;
        }
        match k.sym {
            keysym::ESCAPE if self.sizing.take().is_some() => Reply::redraw(),
            keysym::ESCAPE => {
                self.doc.cancel();
                Reply::redraw()
            }
            keysym::RETURN | keysym::KP_ENTER => {
                self.doc.finish();
                Reply::redraw()
            }
            _ => Reply::none(),
        }
    }

    fn text_key(&mut self, k: &KeyInput) -> Reply {
        if k.sym == keysym::ESCAPE {
            self.doc.text_box = None;
            self.text.clear();
            return Reply::redraw();
        }
        let Some(key) = textbox::key_of(k) else {
            return Reply::none();
        };
        let mut reply = Reply::redraw();
        match &key {
            MultiKey::Edit(EditKey::Copy | EditKey::Cut) => {
                reply.copy = Some(self.text.edit.selected_text());
            }
            MultiKey::Edit(EditKey::Paste) => reply.paste = true,
            _ => {}
        }
        let w = self.doc.text_box.map_or(1, |b| self.text.fit(b).width);
        self.text.key(key, w);
        reply
    }

    fn key_up(&mut self, k: KeyInput, ctx: &Ctx) -> Reply {
        if matches!(k.sym, SHIFT_L | SHIFT_R) {
            self.shift = false;
        }
        if !matches!(k.sym, keysym::ALT_L | keysym::ALT_R) {
            self.alt_arm.interrupt();
            return Reply::none();
        }
        if !self.alt_arm.release_alt() {
            return Reply::none();
        }
        let lay = self.bar_layout(ctx);
        let px = ctx.theme.fonts.size_px;
        self.bar.key(Key::Activate, &lay, &ctx.text, &ctx.theme, px);
        Reply::redraw()
    }

    /// Switches tools: what the old one had going is finished and the text box set.
    pub(crate) fn pick_tool(&mut self, tool: Tool) {
        if tool == self.tool {
            return;
        }
        self.commit_text();
        if !(tool.selects() && self.tool.selects()) {
            self.doc.finish();
        }
        if tool == Tool::Picker {
            self.before_pick = self.tool;
        }
        self.tool = tool;
        if tool == Tool::Text && self.fonts.is_none() {
            self.open_fonts();
        }
    }

    /// Lays the text box's text on the picture.
    pub(crate) fn commit_text(&mut self) {
        let Some(b) = self.doc.text_box else {
            return;
        };
        let b = self.text.fit(b).intersect(&self.doc.img.bounds());
        if self.text.is_empty() || b.is_empty() {
            self.doc.text_box = None;
        } else {
            let cover = self.text.cover(b.width, b.height);
            self.doc.ink_text(b, &cover);
        }
        self.text.clear();
    }

    fn pick_option(&mut self, i: usize) {
        let o = &mut self.doc.opts;
        match options_of(self.tool) {
            Options::None => {}
            Options::Opaque => o.opaque = i == 0,
            Options::Eraser => o.eraser = view::ERASERS[i],
            Options::Zoom => self.magnify = view::MAGNIFY[i],
            Options::Tips => {
                let (tip, size) = view::TIPS[i];
                o.brush = crate::shapes::Brush { tip, size };
            }
            Options::Spray => o.spray = view::SPRAYS[i],
            Options::Widths => o.width = view::WIDTHS[i],
            Options::Fills => o.fill = view::FILLS[i],
        }
    }

    fn effect(&mut self, fx: Effect, ctx: &Ctx) -> Reply {
        match fx {
            Effect::None => Reply::none(),
            Effect::Redraw => Reply::redraw(),
            Effect::Picked(ink, c) => {
                match ink {
                    Ink::Primary => self.doc.primary = c,
                    Ink::Secondary => self.doc.secondary = c,
                }
                self.tool = self.before_pick;
                Reply::redraw()
            }
            Effect::Magnify(at) => {
                let z = if self.zoom == 1 { self.magnify } else { 1 };
                self.held = None;
                self.zoom_to(z, Some(at), ctx);
                Reply::redraw()
            }
        }
    }

    fn mods(&self, m: repriseapp::Mods) -> Mods {
        Mods {
            shift: m.shift || self.shift,
            ctrl: m.ctrl,
        }
    }

    /// Ends the toolbox button's press and a held scrollbar part; true when a bar let go.
    fn release_bars(&mut self) -> bool {
        self.tool_pressed = None;
        self.hscroll.release() | self.vscroll.release()
    }

    /// Ends the drag on the picture at `at`, committed as its button's release commits it.
    fn release_held(&mut self, at: (i32, i32), m: Mods) -> Effect {
        self.held = None;
        let had_box = self.doc.text_box.is_some();
        let fx = self.doc.release(self.tool, at, m);
        if !had_box && let Some(b) = self.doc.text_box {
            let bounds = self.doc.img.bounds();
            let b = self.text.opened(b, bounds.right() - b.x);
            self.doc.text_box = Some(b.intersect(&bounds));
        }
        fx
    }

    /// A modal child covers the window, which will not see the release: the drag on the picture
    /// ends where the pointer last took it, the bars let go and a sizing drag is dropped.
    pub(crate) fn let_go(&mut self) {
        self.release_bars();
        self.sizing = None;
        if let Some((_, at)) = self.held {
            let m = self.mods(repriseapp::Mods::default());
            self.release_held(at, m);
        }
    }

    /// Gives the picture the size the handle's drag reached, as one step Undo takes back; the
    /// new area takes the background colour.
    fn finish_sizing(&mut self, ctx: &Ctx) {
        let Some(d) = self.sizing.take() else {
            return;
        };
        if d.resized() {
            let (w, h) = d.size;
            self.doc
                .replace(self.doc.img.with_size(w, h, self.doc.secondary));
            self.hold_scroll(ctx);
        }
    }

    pub(crate) fn pointer(&mut self, p: PointerInput, ctx: &Ctx) -> Reply {
        let reply = self.pointer_input(p, ctx);
        match p {
            PointerInput::Motion { x, y } => Reply {
                pointer: self.shape_at(x, y, ctx),
                ..reply
            },
            _ => reply,
        }
    }

    /// The pointer's shape at (`x`, `y`): a sizing handle's arrow over it and through its drag.
    fn shape_at(&self, x: i32, y: i32, ctx: &Ctx) -> Shape {
        if let Some(d) = self.sizing {
            return d.handle.shape();
        }
        if self.bar.open.is_some() || self.held.is_some() {
            return Shape::Default;
        }
        let l = self.lay(ctx);
        if !l.view.contains(x, y) || self.bar_at(&l, x, y, ctx).is_some() {
            return Shape::Default;
        }
        sizehandles::hit(&self.handles(&l), x, y).map_or(Shape::Default, |h| h.shape())
    }

    fn pointer_input(&mut self, p: PointerInput, ctx: &Ctx) -> Reply {
        if matches!(p, PointerInput::Press { .. }) {
            self.alt_arm.interrupt();
        }
        let lay = self.bar_layout(ctx);
        let px = ctx.theme.fonts.size_px;
        let l = self.lay(ctx);
        match p {
            PointerInput::Press {
                x,
                y,
                button,
                time,
                mods,
            } if button == BTN_LEFT || button == BTN_RIGHT => {
                if button == BTN_LEFT {
                    match self.bar.press(x, y, &lay, &ctx.text, &ctx.theme, px) {
                        Action::Chosen { title, row, .. } => {
                            return self.command(title, row, ctx);
                        }
                        Action::Redraw => return Reply::redraw(),
                        Action::Passed => {
                            return Reply {
                                redraw: true,
                                ..self.pointer(p, ctx)
                            };
                        }
                        Action::Ignored => {}
                    }
                    if let Some((bar, r)) = self.status(&l, ctx)
                        && statusbar::hit_grip(&bar, r, x, y)
                    {
                        return Reply::resize();
                    }
                }
                if l.tools.contains(x, y) {
                    let tb = self.toolbox(1, ctx);
                    match toolbox::hit(&toolbox::layout(&tb, l.tools), x, y) {
                        Some(Hit::Tool(i)) => {
                            self.tool_pressed = Some(i);
                            self.pick_tool(doc::TOOLS[i]);
                        }
                        Some(Hit::Option(i)) => self.pick_option(i),
                        None => return Reply::none(),
                    }
                    return Reply::redraw();
                }
                if l.colors.contains(x, y) {
                    let pal = self.palette();
                    if let Some(i) = palette::hit(&palette::layout(&pal, l.colors), x, y) {
                        let c = self.wells[i].argb();
                        let secondary = button != BTN_LEFT;
                        if secondary {
                            self.doc.secondary = c;
                        } else {
                            self.doc.primary = c;
                        }
                        let click = ctx.theme.metrics.click(x, y, time);
                        if self.well_clicks.press((i, button), click) {
                            self.commit_text();
                            self.open_colors(secondary, Some(i));
                            return Reply::redraw_all();
                        }
                        return Reply::redraw();
                    }
                    return Reply::none();
                }
                if let Some((across, hit)) = self.bar_at(&l, x, y, ctx) {
                    if button == BTN_LEFT {
                        if across {
                            self.hscroll.top = self.hscroll.press(hit, l.view.width);
                        } else {
                            self.vscroll.top = self.vscroll.press(hit, l.view.height);
                        }
                        self.hold_scroll(ctx);
                        return Reply {
                            redraw: true,
                            tick: self.hscroll.ticking() || self.vscroll.ticking(),
                            ..Reply::none()
                        };
                    }
                    return Reply::none();
                }
                if !l.view.contains(x, y) || self.held.is_some() || self.sizing.is_some() {
                    return Reply::none();
                }
                if button == BTN_LEFT
                    && let Some(h) = sizehandles::hit(&self.handles(&l), x, y)
                {
                    self.commit_text();
                    self.doc.finish();
                    let size = (self.doc.img.w, self.doc.img.h);
                    self.sizing = Some(sizehandles::Drag::new(h, (x, y), size));
                    return Reply::redraw();
                }
                let at = self.to_picture(&l, x, y);
                if self.doc.text_box.is_some() {
                    let inside = self
                        .doc
                        .text_box
                        .is_some_and(|b| self.text.fit(b).contains(at.0, at.1));
                    if inside && self.tool == Tool::Text {
                        return Reply::none();
                    }
                    self.commit_text();
                    if self.tool == Tool::Text {
                        return Reply::redraw();
                    }
                }
                let double = self.clicks.press((), ctx.theme.metrics.click(x, y, time));
                let ink = if button == BTN_LEFT {
                    Ink::Primary
                } else {
                    Ink::Secondary
                };
                self.held = Some((button, at));
                let fx = self.doc.press(self.tool, at, ink, self.mods(mods), double);
                let reply = self.effect(fx, ctx);
                Reply {
                    tick: self.doc.spraying(),
                    ..reply
                }
            }
            PointerInput::Release {
                x, y, button, mods, ..
            } => {
                let bars = self.release_bars();
                if button == BTN_LEFT && self.sizing.is_some() {
                    self.finish_sizing(ctx);
                    return Reply::redraw();
                }
                if self.held.map(|(b, _)| b) != Some(button) {
                    return if bars { Reply::redraw() } else { Reply::none() };
                }
                let at = self.to_picture(&l, x, y);
                let fx = self.release_held(at, self.mods(mods));
                self.effect(fx, ctx)
            }
            PointerInput::Motion { x, y } => {
                if self.bar.open.is_some() {
                    self.bar.motion(x, y, &lay, &ctx.text, &ctx.theme, px);
                    return Reply::redraw();
                }
                let title = self.bar.hover(x, y, &lay);
                if let Some(d) = &mut self.sizing {
                    let changed = d.motion((x, y), self.zoom, dialogs::MAX_SIDE);
                    return if changed {
                        Reply::redraw()
                    } else {
                        Reply::none()
                    };
                }
                if self.hscroll.holding()
                    && let Some(b) = l.hbar
                {
                    let parts = self.hbar_parts(b, &l, ctx);
                    if let Some(to) =
                        self.hscroll
                            .drag(&parts, (x, y), l.view.width, self.extent().0)
                    {
                        self.hscroll.top = to;
                    }
                    return Reply::redraw();
                }
                if self.vscroll.holding()
                    && let Some(b) = l.vbar
                {
                    let parts = self.vbar_parts(b, &l, ctx);
                    if let Some(to) =
                        self.vscroll
                            .drag(&parts, (x, y), l.view.height, self.extent().1)
                    {
                        self.vscroll.top = to;
                    }
                    return Reply::redraw();
                }
                let over = self.bar_at(&l, x, y, ctx);
                let lit = self.hscroll.motion(over.filter(|o| o.0).map(|o| o.1))
                    | self.vscroll.motion(over.filter(|o| !o.0).map(|o| o.1));
                let tb = self.toolbox(1, ctx);
                let hot = match toolbox::hit(&toolbox::layout(&tb, l.tools), x, y) {
                    Some(Hit::Tool(i)) => Some(i),
                    _ => None,
                };
                let tool_lit = std::mem::replace(&mut self.tool_hot, hot) != hot;
                let at = self.to_picture(&l, x, y);
                let on = (l.view.contains(x, y) || self.held.is_some()).then_some(at);
                let moved = std::mem::replace(&mut self.hover, on) != on;
                let drew = if let Some((_, last)) = &mut self.held {
                    *last = at;
                    let m = self.mods(repriseapp::Mods::default());
                    self.doc.motion(self.tool, at, m) != Effect::None
                } else {
                    false
                };
                let styled = (lit || tool_lit || title) && ctx.styled();
                if drew || moved || styled {
                    Reply::redraw()
                } else {
                    Reply::none()
                }
            }
            PointerInput::Scroll { rows, .. } => {
                self.vscroll.top += rows * LINE_H;
                self.hold_scroll(ctx);
                Reply::redraw()
            }
            PointerInput::ScrollAcross { cols, .. } => {
                self.hscroll.top += cols * LINE_H;
                self.hold_scroll(ctx);
                Reply::redraw()
            }
            PointerInput::Leave => {
                let title = self.bar.leave();
                let bars = self.hscroll.leave() | self.vscroll.leave();
                let hot = self.tool_hot.take().is_some();
                let hover = self.hover.take().is_some() && self.held.is_none();
                if hover || ((title || bars || hot) && ctx.styled()) {
                    Reply::redraw()
                } else {
                    Reply::none()
                }
            }
            _ => Reply::none(),
        }
    }
}
