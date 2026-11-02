//! Keys and the pointer in the main window.

use super::*;
use menu::{row, title};
use repriseapp::{BTN_LEFT, KeyInput, PointerInput};
use reprisekit::controls::imageview::LINE;
use reprisekit::controls::menubar::{Action, Key};
use reprisekit::controls::statusbar;
use reprisekit::pointer::{Nav, WHEEL_ROWS, nav_button};
use reprisekit::textedit::keysym;

/// The menu row key `sym` stands for, with Ctrl held or not.
pub(crate) fn accel(sym: u32, ctrl: bool) -> Option<(usize, usize)> {
    if ctrl {
        return Some(match sym | 0x20 {
            0x6f => (title::FILE, row::OPEN),
            0x63 => (title::EDIT, row::COPY),
            0x72 => (title::IMAGE, row::ROTATE_CW),
            0x6c => (title::IMAGE, row::ROTATE_CCW),
            0x30 => (title::VIEW, row::FIT),
            0x31 => (title::VIEW, row::ACTUAL),
            0x2b | 0x3d | keysym::KP_ADD => (title::VIEW, row::ZOOM_IN),
            0x2d | keysym::KP_SUBTRACT => (title::VIEW, row::ZOOM_OUT),
            _ => return None,
        });
    }
    Some(match sym {
        keysym::DELETE | keysym::KP_DELETE => (title::EDIT, row::DELETE),
        keysym::LEFT
        | keysym::KP_LEFT
        | keysym::PAGE_UP
        | keysym::KP_PAGE_UP
        | keysym::BACKSPACE => (title::VIEW, row::PREVIOUS),
        keysym::RIGHT
        | keysym::KP_RIGHT
        | keysym::PAGE_DOWN
        | keysym::KP_PAGE_DOWN
        | keysym::SPACE => (title::VIEW, row::NEXT),
        keysym::HOME | keysym::KP_HOME => (title::VIEW, row::FIRST),
        keysym::END | keysym::KP_END => (title::VIEW, row::LAST),
        0x2b | 0x3d | keysym::KP_ADD => (title::VIEW, row::ZOOM_IN),
        0x2d | keysym::KP_SUBTRACT => (title::VIEW, row::ZOOM_OUT),
        keysym::F11 => (title::VIEW, row::FULL_SCREEN),
        keysym::F5 => (title::VIEW, row::SLIDESHOW),
        _ => return None,
    })
}

impl Viewer {
    fn key(&mut self, k: KeyInput, ctx: &Ctx) -> Reply {
        if matches!(k.sym, keysym::CONTROL_L | keysym::CONTROL_R) {
            self.ctrl = true;
            return Reply::none();
        }
        let is_alt = k.sym == keysym::ALT_L || k.sym == keysym::ALT_R;
        if is_alt && !self.full {
            self.alt_arm.press_alt();
            return Reply::none();
        }
        self.alt_arm.interrupt();
        if !self.full
            && let Some(r) = self.bar_key(&k, ctx)
        {
            return r;
        }
        if k.sym == keysym::ESCAPE {
            return match (self.slide_due.is_some(), self.full) {
                (_, true) => self.full_screen(false),
                (true, false) => self.slideshow(false),
                _ => Reply::none(),
            };
        }
        if matches!(k.sym, keysym::DELETE | keysym::KP_DELETE) && k.mods.shift {
            return self.ask_delete(true);
        }
        let lay = self.view_layout(ctx);
        let pan = match k.sym {
            keysym::UP | keysym::KP_UP => Some((0, -LINE)),
            keysym::DOWN | keysym::KP_DOWN => Some((0, LINE)),
            _ => None,
        };
        if let Some((dx, dy)) = pan {
            return match self.view.pan(dx, dy, &lay) {
                true => Reply::redraw(),
                false => Reply::none(),
            };
        }
        match accel(k.sym, k.mods.ctrl) {
            Some((t, r)) => self.command(t, r, ctx),
            None => Reply::none(),
        }
    }

    /// The menu bar's keys: F10, Escape and the arrows while it is active, Alt+letter.
    fn bar_key(&mut self, k: &KeyInput, ctx: &Ctx) -> Option<Reply> {
        let bar_key = match k.sym {
            keysym::F10 if !k.mods.shift => Key::Activate,
            keysym::ESCAPE => Key::Escape,
            keysym::LEFT | keysym::KP_LEFT => Key::Left,
            keysym::RIGHT | keysym::KP_RIGHT => Key::Right,
            keysym::UP | keysym::KP_UP => Key::Up,
            keysym::DOWN | keysym::KP_DOWN => Key::Down,
            keysym::RETURN | keysym::KP_ENTER => Key::Enter,
            _ => k
                .utf8
                .as_deref()
                .and_then(|s| s.chars().next())
                .filter(|c| c.is_alphanumeric())
                .map(|c| Key::Char(c, k.mods.alt))?,
        };
        let lay = self.bar_layout(ctx);
        let px = ctx.theme.fonts.size_px;
        match self.bar.key(bar_key, &lay, &ctx.text, &ctx.theme, px) {
            Action::Chosen { title, row, .. } => Some(self.command(title, row, ctx)),
            Action::Redraw => Some(Reply::redraw()),
            Action::Ignored => None,
        }
    }

    fn key_up(&mut self, k: KeyInput, ctx: &Ctx) -> Reply {
        if matches!(k.sym, keysym::CONTROL_L | keysym::CONTROL_R) {
            self.ctrl = false;
            return Reply::none();
        }
        if k.sym != keysym::ALT_L && k.sym != keysym::ALT_R {
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

    fn lit(&self, changed: bool, ctx: &Ctx) -> Reply {
        if changed && ctx.styled() {
            Reply::redraw()
        } else {
            Reply::none()
        }
    }

    pub(crate) fn pointer(&mut self, p: PointerInput, ctx: &Ctx) -> Reply {
        if matches!(p, PointerInput::Press { .. }) {
            self.alt_arm.interrupt();
        }
        let blay = self.bar_layout(ctx);
        let (m, px) = (&ctx.theme, ctx.theme.fonts.size_px);
        let lay = self.view_layout(ctx);
        match p {
            PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                time,
                ..
            } => {
                if !self.full {
                    match self.bar.press(x, y, &blay, &ctx.text, m, px) {
                        Action::Chosen { title, row, .. } => return self.command(title, row, ctx),
                        Action::Redraw => return Reply::redraw(),
                        Action::Ignored => {}
                    }
                    if let Some((bar, r)) = self.status(ctx)
                        && statusbar::hit_grip(&bar, r, x, y)
                    {
                        return Reply::resize();
                    }
                }
                let on_bars = self.view.bar_at(&lay, x, y).is_some();
                if !on_bars
                    && lay.view.contains(x, y)
                    && self.clicks.press((), m.metrics.click(x, y, time))
                {
                    self.view.release();
                    return self.full_screen(!self.full);
                }
                match self.view.press(x, y, &lay) {
                    true => Reply::redraw(),
                    false => Reply::none(),
                }
            }
            PointerInput::Press { button, .. } => match nav_button(button) {
                Some(Nav::Back) => self.go(Step::Previous),
                Some(Nav::Forward) => self.go(Step::Next),
                None => Reply::none(),
            },
            PointerInput::Release { .. } => {
                let lit = self.view.release();
                self.lit(lit, ctx)
            }
            PointerInput::Leave => {
                let title = self.bar.leave();
                let lit = self.view.leave() || title;
                self.lit(lit, ctx)
            }
            PointerInput::Motion { x, y } => {
                if !self.full && self.bar.open.is_some() {
                    self.bar.motion(x, y, &blay, &ctx.text, m, px);
                    return Reply::redraw();
                }
                let title = !self.full && self.bar.hover(x, y, &blay);
                let dragging = self.view.dragging();
                let moved = self.view.motion(x, y, &lay);
                if dragging && moved {
                    return Reply::redraw();
                }
                self.lit(moved || title, ctx)
            }
            PointerInput::Scroll { x, y, rows } => self.wheel(x, y, rows, ctx),
        }
    }

    /// The wheel steps through the folder a notch at a time; with Ctrl held it zooms at the
    /// pointer.
    fn wheel(&mut self, x: i32, y: i32, rows: i32, ctx: &Ctx) -> Reply {
        self.wheel_rows += rows;
        if self.wheel_rows.abs() < WHEEL_ROWS {
            return Reply::none();
        }
        let back = self.wheel_rows < 0;
        self.wheel_rows = 0;
        if self.ctrl {
            if self.picture.is_none() {
                return Reply::none();
            }
            self.zoom_step(back, Some((x, y)), ctx);
            return Reply::redraw();
        }
        self.go(if back { Step::Previous } else { Step::Next })
    }

    pub(crate) fn main_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        match ev {
            Event::Key(k) => self.key(k, ctx),
            Event::KeyUp(k) => self.key_up(k, ctx),
            Event::Pointer(p) => self.pointer(p, ctx),
            Event::Wake => match self.drain() {
                true => Reply::redraw(),
                false => Reply::none(),
            },
            Event::CloseRequest => Reply {
                quit: true,
                ..Reply::none()
            },
            Event::Focus(false) => {
                self.alt_arm.interrupt();
                self.ctrl = false;
                Reply::redraw()
            }
            Event::Resized | Event::Focus(_) => Reply::redraw(),
            _ => Reply::none(),
        }
    }
}
