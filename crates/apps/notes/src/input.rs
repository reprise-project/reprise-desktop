//! Keys, the pointer and the right-click menu in a note's window.

use super::*;
use repriseapp::{BTN_LEFT, BTN_RIGHT, KeyInput, PointerInput};
use reprisekit::controls::menu::MenuItem;
use reprisekit::controls::multiedit::MultiKey;
use reprisekit::controls::stickynote::Part;
use reprisekit::textedit::{EditKey, keysym};

/// The rows the menu adds under the edit ones: the colours, then New Note and Delete Note.
const MENU_NEW: usize = Colour::ALL.len();
const MENU_DELETE: usize = MENU_NEW + 1;

/// The menu's own rows for a note in `colour`: that one's radio set.
pub(crate) fn menu_rows(colour: Colour) -> Vec<MenuItem> {
    let mut rows: Vec<MenuItem> = (Colour::ALL.into_iter())
        .map(|c| MenuItem::item(&c.label()).radio(true).checked(c == colour))
        .collect();
    rows.push(MenuItem::item(&tr!("menu-new")));
    rows.push(MenuItem::item(&tr!("menu-delete")));
    rows
}

impl Notes {
    pub(crate) fn note_event(&mut self, win: WinId, ev: Event, ctx: &Ctx) -> Reply {
        match ev {
            Event::Key(k) => self.key(win, &k, ctx),
            Event::Pointer(p) => self.pointer(win, p, ctx),
            Event::Pasted(text) => {
                let Some(n) = self.note_mut(win) else {
                    return Reply::none();
                };
                if n.edit.paste(&text) {
                    self.edited(win, ctx);
                }
                Reply::redraw()
            }
            Event::Focus(on) => {
                if !on && self.menu_on(win) {
                    self.menu = None;
                }
                if let Some(n) = self.note_mut(win) {
                    n.focused = on;
                }
                Reply::redraw()
            }
            Event::Resized => {
                if let Some(n) = self.note_mut(win)
                    && !ctx.maximized
                    && n.size != ctx.size
                {
                    n.size = ctx.size;
                    self.changed(win);
                }
                Reply::redraw()
            }
            Event::Tick => self.select_drag(win, true, ctx),
            Event::Blink => Reply::redraw(),
            _ => Reply::none(),
        }
    }

    /// The text of `win` changed: the caret stays in view and the write waits for a pause.
    fn edited(&mut self, win: WinId, ctx: &Ctx) {
        self.scroll_to_caret(win, ctx);
        self.changed(win);
    }

    fn scroll_to_caret(&mut self, win: WinId, ctx: &Ctx) {
        let Some(n) = self.note(win) else {
            return;
        };
        let lay = self.edit_layout(n, ctx);
        let m = self.metrics;
        if let Some(n) = self.note_mut(win) {
            n.view.follow(&ctx.text, &m, &n.edit, &lay, 1);
        }
    }

    fn key(&mut self, win: WinId, k: &KeyInput, ctx: &Ctx) -> Reply {
        if self.menu_on(win) {
            let cx = self.menu_ctx(ctx);
            let Some(menu) = &mut self.menu else {
                return Reply::none();
            };
            let step = menu.key(k.sym, k.utf8.as_deref(), &cx);
            return self.menu_step(win, step, ctx);
        }
        if editmenu::opens(k.sym, k.mods.shift) {
            let at = self.caret_point(win, ctx);
            self.open_menu(win, at, true, ctx);
            return Reply::redraw();
        }
        if k.mods.ctrl && !k.mods.alt && matches!(k.sym, 0x6e | 0x4e) {
            let colour = self.note(win).map_or(Colour::default(), |n| n.colour);
            self.new_note(colour);
            return Reply::redraw_all();
        }
        let Some(n) = self.note(win) else {
            return Reply::none();
        };
        let rows = self.edit_layout(n, ctx).rows;
        let select = k.mods.shift;
        let key = match k.sym {
            keysym::UP | keysym::KP_UP => Some(MultiKey::Up { select }),
            keysym::DOWN | keysym::KP_DOWN => Some(MultiKey::Down { select }),
            keysym::PAGE_UP | keysym::KP_PAGE_UP => Some(MultiKey::PageUp { select, rows }),
            keysym::PAGE_DOWN | keysym::KP_PAGE_DOWN => Some(MultiKey::PageDown { select, rows }),
            keysym::HOME | keysym::KP_HOME if k.mods.ctrl => Some(MultiKey::DocHome { select }),
            keysym::END | keysym::KP_END if k.mods.ctrl => Some(MultiKey::DocEnd { select }),
            keysym::RETURN | keysym::KP_ENTER => Some(MultiKey::Enter),
            keysym::TAB => Some(MultiKey::Tab),
            _ => k.edit.clone().map(MultiKey::Edit),
        };
        match key {
            Some(key) => self.edit_key(win, key, ctx),
            None => Reply::none(),
        }
    }

    /// `key` in note `win`'s text, with the clipboard half of a copy, cut or paste.
    fn edit_key(&mut self, win: WinId, key: MultiKey, ctx: &Ctx) -> Reply {
        let m = self.metrics;
        let Some(n) = self.note_mut(win) else {
            return Reply::none();
        };
        let mut reply = Reply::redraw();
        match &key {
            MultiKey::Edit(EditKey::Copy | EditKey::Cut) => {
                reply.copy = Some(n.edit.selected_text()).filter(|s| !s.is_empty());
            }
            MultiKey::Edit(EditKey::Paste) => reply.paste = true,
            _ => {}
        }
        if n.edit.key(key, &ctx.text, &m) {
            self.edited(win, ctx);
        } else {
            self.scroll_to_caret(win, ctx);
        }
        reply
    }

    /// Below the caret's line, where Shift+F10 and the Menu key open the menu.
    fn caret_point(&self, win: WinId, ctx: &Ctx) -> (i32, i32) {
        let Some(n) = self.note(win) else {
            return (0, 0);
        };
        let el = self.edit_layout(n, ctx);
        let caret = n.edit.caret();
        let x = multiedit::x_of(&ctx.text, &self.metrics, &n.edit, caret) - n.view.left;
        let row = caret.line.saturating_sub(n.view.top) as i32 + 1;
        (
            (el.inner.x + 1 + x).clamp(el.inner.x, el.inner.right()),
            (el.inner.y + 1 + row * self.metrics.line_h).clamp(el.inner.y, el.inner.bottom()),
        )
    }

    pub(crate) fn open_menu(&mut self, win: WinId, at: (i32, i32), by_keyboard: bool, ctx: &Ctx) {
        let Some(n) = self.note(win) else {
            return;
        };
        let live = editmenu::Live {
            undo: n.edit.can_undo(),
            redo: n.edit.can_redo(),
            selection: n.edit.has_selection(),
            paste: repriseapp::clipboard_has_text(),
            secret: false,
        };
        let rows = menu_rows(n.colour);
        let menu = editmenu::Menu::open_with(live, rows, at, &self.menu_ctx(ctx), by_keyboard);
        self.menu = Some(menu);
        self.menu_win = win;
    }

    fn menu_step(&mut self, win: WinId, step: editmenu::Step, ctx: &Ctx) -> Reply {
        match step {
            editmenu::Step::None => return Reply::none(),
            editmenu::Step::Redraw => return Reply::redraw(),
            _ => {}
        }
        self.menu = None;
        match step {
            editmenu::Step::Run(cmd) => self.edit_command(win, cmd, ctx),
            editmenu::Step::Extra(MENU_NEW) => {
                let colour = self.note(win).map_or(Colour::default(), |n| n.colour);
                self.new_note(colour);
                Reply::redraw_all()
            }
            editmenu::Step::Extra(MENU_DELETE) => self.ask_delete(win),
            editmenu::Step::Extra(i) => match Colour::ALL.get(i) {
                Some(&c) => {
                    if let Some(n) = self.note_mut(win) {
                        n.colour = c;
                    }
                    self.changed(win);
                    Reply::redraw()
                }
                None => Reply::redraw(),
            },
            _ => Reply::redraw(),
        }
    }

    fn edit_command(&mut self, win: WinId, cmd: editmenu::Command, ctx: &Ctx) -> Reply {
        let key = match cmd {
            editmenu::Command::Undo => EditKey::Undo,
            editmenu::Command::Redo => EditKey::Redo,
            editmenu::Command::Cut => EditKey::Cut,
            editmenu::Command::Copy => EditKey::Copy,
            editmenu::Command::Paste => EditKey::Paste,
            editmenu::Command::Delete => EditKey::Delete { word: false },
            editmenu::Command::SelectAll => EditKey::SelectAll,
        };
        self.edit_key(win, MultiKey::Edit(key), ctx)
    }

    fn pointer(&mut self, win: WinId, p: PointerInput, ctx: &Ctx) -> Reply {
        if self.menu_on(win) {
            let cx = self.menu_ctx(ctx);
            let Some(menu) = &mut self.menu else {
                return Reply::none();
            };
            return match p {
                PointerInput::Press { x, y, .. } => {
                    let step = menu.press(x, y, &cx);
                    self.menu_step(win, step, ctx)
                }
                PointerInput::Motion { x, y } if menu.motion(x, y, &cx) => Reply::redraw(),
                _ => Reply::none(),
            };
        }
        let lay = stickynote::layout(Notes::client(ctx));
        let Some(n) = self.note(win) else {
            return Reply::none();
        };
        let el = self.edit_layout(n, ctx);
        let m = self.metrics;
        let text = &ctx.text;
        let Some(n) = self.note_mut(win) else {
            return Reply::none();
        };
        match p {
            PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                time,
                ..
            } => {
                let over = stickynote::hit(&lay, x, y);
                if n.strip.press(over) {
                    return Reply::redraw();
                }
                if el.inner.contains(x, y) {
                    let pos = multiedit::pos_at(text, &m, &n.edit, &el, n.view, x, y, 1);
                    if n.clicks.press((), ctx.theme.metrics.click(x, y, time)) {
                        n.edit.select_word_at(pos);
                    } else {
                        n.edit.place(pos, false);
                        n.dragging = Some((x, y));
                    }
                    return Reply::redraw();
                }
                if let Some((bar, hit)) = multiedit::bar_hit(text, &m, &n.edit, &el, n.view, x, y) {
                    n.thumb.press(bar.axis(), hit);
                    n.view = n.view.press_bar(text, &m, &n.edit, &el, bar, hit);
                    return Reply::redraw();
                }
                Reply::none()
            }
            PointerInput::Press {
                x,
                y,
                button: BTN_RIGHT,
                ..
            } => {
                if el.inner.contains(x, y) {
                    let pos = multiedit::pos_at(text, &m, &n.edit, &el, n.view, x, y, 1);
                    let (start, end) = n.edit.selection();
                    if !(n.edit.has_selection() && (start..=end).contains(&pos)) {
                        n.edit.place(pos, false);
                    }
                }
                n.dragging = None;
                n.thumb.release();
                n.view.bars.release_bar();
                self.open_menu(win, (x, y), false, ctx);
                Reply::redraw()
            }
            PointerInput::Release { x, y, button, .. } => {
                n.dragging = None;
                n.thumb.release();
                n.view.bars.release_bar();
                let clicked = match button {
                    BTN_LEFT => n.strip.release(stickynote::hit(&lay, x, y)),
                    _ => None,
                };
                match clicked {
                    Some(Part::New) => {
                        let colour = n.colour;
                        self.new_note(colour);
                        Reply::redraw_all()
                    }
                    Some(Part::Delete) => self.ask_delete(win),
                    None => Reply::redraw(),
                }
            }
            PointerInput::Motion { x, y } => {
                let lit = n.strip.motion(stickynote::hit(&lay, x, y));
                if n.thumb.holding() {
                    if let Some(v) =
                        multiedit::drag_bar(text, &m, &n.edit, &el, n.view, &n.thumb, (x, y))
                    {
                        n.view = v;
                    }
                    return Reply::redraw();
                }
                if n.dragging.is_some() {
                    n.dragging = Some((x, y));
                    return self.select_drag(win, false, ctx);
                }
                if lit { Reply::redraw() } else { Reply::none() }
            }
            PointerInput::Scroll { rows, .. } => {
                if n.view.scroll_lines(rows as i64, &n.edit, el.rows) {
                    Reply::redraw()
                } else {
                    Reply::none()
                }
            }
            PointerInput::Leave => {
                if n.strip.leave() {
                    Reply::redraw()
                } else {
                    Reply::none()
                }
            }
            _ => Reply::none(),
        }
    }

    /// The held selection drag in `win` follows the pointer, scrolling while it rests outside.
    fn select_drag(&mut self, win: WinId, tick: bool, ctx: &Ctx) -> Reply {
        let Some(n) = self.note(win) else {
            return Reply::none();
        };
        let el = self.edit_layout(n, ctx);
        let m = self.metrics;
        let Some(n) = self.note_mut(win) else {
            return Reply::none();
        };
        let Some((x, y)) = n.dragging else {
            return Reply::none();
        };
        let step =
            multiedit::select_drag(&ctx.text, &m, &mut n.edit, &el, &mut n.view, x, y, 1, tick);
        Reply {
            redraw: step.changed,
            tick: step.scrolling,
            ..Reply::none()
        }
    }
}
