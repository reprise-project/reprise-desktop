//! Character Map: the window around `reprisekit::controls::charmap`, loading the font picked and
//! putting characters on the clipboard.

reprisekit::strings!("charmap");

use repriseapp::keys::{dialog_key, form_input};
use repriseapp::{App, BTN_LEFT, Ctx, Event, Extended, KeyInput, PointerInput, Reply, WinId};
use reprisekit::access::{ROOT, Tree};
use reprisekit::classic::widgets::BarHit;
use reprisekit::controls::charmap::{self, CharMap, Hit, Step};
use reprisekit::controls::form::{self, Form};
use reprisekit::controls::scrollbar::ThumbDrag;
use reprisekit::look::Paint;
use reprisekit::pointer::{Axis, DoubleClick};
use reprisekit::raster::Canvas;
use reprisekit::text::TextRenderer;
use reprisekit::textedit::{EditKey, keysym};
use reprisekit::theme::FontConfig;
use reprisekit::tr;

/// The rows the wheel scrolls per notch.
const WHEEL_ROWS: i64 = 3;

struct CharMapWindow {
    map: CharMap,
    fonts: Vec<String>,
    font: usize,
    /// The font picked, the grid and the large character drawn in it.
    glyphs: TextRenderer,
    form: Form,
    /// The grid has the keyboard rather than one of the form's controls.
    grid_focus: bool,
    clicks: DoubleClick<usize>,
    thumb: ThumbDrag,
}

fn load(family: &str) -> TextRenderer {
    TextRenderer::load(&FontConfig {
        family: vec![family.to_string()],
        antialias: true,
        ..FontConfig::default()
    })
}

impl CharMapWindow {
    /// The window on `fonts`, `font` picked.
    fn new(fonts: Vec<String>, font: usize) -> CharMapWindow {
        let glyphs = fonts.get(font).map_or_else(|| load(""), |f| load(f));
        let map = CharMap::new(glyphs.chars());
        CharMapWindow {
            form: charmap::form(&fonts, font, &map),
            map,
            fonts,
            font,
            glyphs,
            grid_focus: true,
            clicks: DoubleClick::default(),
            thumb: ThumbDrag::default(),
        }
    }

    fn set_font(&mut self, font: usize) {
        let Some(family) = self.fonts.get(font) else {
            return;
        };
        self.font = font;
        self.glyphs = load(family);
        self.map.set_chars(self.glyphs.chars());
        charmap::follow(&mut self.form, &self.map);
    }

    fn grid(&self, ctx: &Ctx) -> Option<charmap::Layout> {
        charmap::grid_layout(&self.form, &ctx.forms().controls())
    }

    fn rows(&self, ctx: &Ctx) -> usize {
        self.grid(ctx).map_or(1, |l| l.rows)
    }

    fn focus_grid(&mut self) {
        self.grid_focus = true;
        self.form.set_focus(None);
    }

    fn append_picked(&mut self) -> Reply {
        if let Some(ch) = self.map.picked() {
            charmap::append(&mut self.form, ch);
        }
        Reply::redraw()
    }

    fn copy(&self) -> Reply {
        let text = self.form.edit_text(charmap::TO_COPY);
        Reply {
            copy: (!text.is_empty()).then(|| text.to_string()),
            ..Reply::redraw()
        }
    }

    fn form_event(&mut self, ev: form::Event) -> Reply {
        match ev {
            form::Event::Changed(charmap::FONT) => {
                if let Some(i) = self.form.combo_selected(charmap::FONT) {
                    self.set_font(i);
                }
                Reply::redraw()
            }
            form::Event::Changed(charmap::BLOCK) => {
                self.map.set_block(charmap::picked_block(&self.form));
                self.map.top = 0;
                charmap::follow(&mut self.form, &self.map);
                Reply::redraw()
            }
            form::Event::Changed(charmap::SEARCH) => {
                let query = self.form.edit_text(charmap::SEARCH).to_string();
                self.map.set_query(&query);
                self.map.top = 0;
                charmap::follow(&mut self.form, &self.map);
                Reply::redraw()
            }
            form::Event::Pressed(charmap::SELECT) => self.append_picked(),
            form::Event::Pressed(charmap::COPY) => self.copy(),
            form::Event::None => Reply::none(),
            _ => Reply::redraw(),
        }
    }

    fn grid_key(&mut self, k: &KeyInput, ctx: &Ctx) -> Option<Reply> {
        let step = match k.sym {
            keysym::LEFT | keysym::KP_LEFT => Step::Left,
            keysym::RIGHT | keysym::KP_RIGHT => Step::Right,
            keysym::UP | keysym::KP_UP => Step::Up,
            keysym::DOWN | keysym::KP_DOWN => Step::Down,
            keysym::PAGE_UP | keysym::KP_PAGE_UP => Step::PageUp,
            keysym::PAGE_DOWN | keysym::KP_PAGE_DOWN => Step::PageDown,
            keysym::HOME | keysym::KP_HOME => Step::Home,
            keysym::END | keysym::KP_END => Step::End,
            keysym::RETURN | keysym::KP_ENTER | keysym::SPACE => return Some(self.append_picked()),
            _ if k.edit == Some(EditKey::Copy) => {
                return Some(Reply {
                    copy: self.map.picked().map(String::from),
                    ..Reply::none()
                });
            }
            _ => return None,
        };
        let rows = self.rows(ctx);
        self.map.step(step, rows);
        charmap::follow(&mut self.form, &self.map);
        Some(Reply::redraw())
    }

    fn key(&mut self, k: &KeyInput, ev: &Event, ctx: &Ctx) -> Reply {
        let tab = matches!(k.sym, keysym::TAB | keysym::ISO_LEFT_TAB);
        let back = k.sym == keysym::ISO_LEFT_TAB || k.mods.shift;
        if self.grid_focus && !k.mods.alt {
            if tab {
                self.grid_focus = false;
                self.form.focus_edge(back);
                return Reply::redraw();
            }
            if let Some(r) = self.grid_key(k, ctx) {
                return r;
            }
        }
        // Tab past the form's last control (or Shift+Tab before its first) reaches the grid.
        if tab && !self.grid_focus && !self.form.popup_open() {
            if !self.form.tab_no_wrap(back) {
                self.focus_grid();
            }
            return Reply::redraw();
        }
        if self.grid_focus && dialog_key(k).is_some_and(|d| matches!(d, form::Key::Access(_))) {
            self.grid_focus = false;
        }
        let reply = match form_input(&mut self.form, ev, ctx) {
            Some(e) => self.form_event(e),
            None => Reply::none(),
        };
        if self.form.focus().is_some() {
            self.grid_focus = false;
        }
        reply
    }

    fn pointer(&mut self, p: &PointerInput, ev: &Event, ctx: &Ctx) -> Reply {
        let Some(lay) = self.grid(ctx) else {
            return Reply::none();
        };
        let cm = ctx.forms().controls();
        let popup = self.form.popup_open();
        match *p {
            PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                time,
                ..
            } if !popup => match charmap::hit(&self.map, &lay, &cm, x, y) {
                Some(Hit::Cell(i)) => {
                    self.focus_grid();
                    self.map.selected = Some(i);
                    charmap::follow(&mut self.form, &self.map);
                    if self.clicks.press(i, ctx.theme.metrics.click(x, y, time)) {
                        self.append_picked();
                    }
                    return Reply::redraw();
                }
                Some(Hit::Bar(hit)) => {
                    self.thumb.press(Axis::Vertical, hit);
                    let (rows, page) = (lay.rows, lay.rows as i64);
                    match hit {
                        BarHit::Back => self.map.scroll(-1, rows),
                        BarHit::Forward => self.map.scroll(1, rows),
                        BarHit::PageBack => self.map.scroll(-page, rows),
                        BarHit::PageForward => self.map.scroll(page, rows),
                        BarHit::Thumb(_) | BarHit::Track => false,
                    };
                    return Reply::redraw();
                }
                None if lay.frame.contains(x, y) => {
                    self.focus_grid();
                    return Reply::redraw();
                }
                None => {}
            },
            PointerInput::Motion { x, y } if self.thumb.holding() => {
                let parts = lay.bar_parts(&self.map, cm.thumb_min);
                let total = self.map.rows();
                if let Some(top) = self
                    .thumb
                    .pos(Axis::Vertical, &parts, (x, y), lay.rows, total)
                {
                    self.map.scroll_to(top, lay.rows);
                }
                return Reply::redraw();
            }
            PointerInput::Release {
                button: BTN_LEFT, ..
            } if self.thumb.release() => return Reply::redraw(),
            PointerInput::Scroll { x, y, rows } if !popup && lay.frame.contains(x, y) => {
                return if self.map.scroll(i64::from(rows) * WHEEL_ROWS, lay.rows) {
                    Reply::redraw()
                } else {
                    Reply::none()
                };
            }
            _ => {}
        }
        let reply = match form_input(&mut self.form, ev, ctx) {
            Some(e) => self.form_event(e),
            None => Reply::none(),
        };
        if self.form.focus().is_some() {
            self.grid_focus = false;
        }
        reply
    }
}

impl App for CharMapWindow {
    fn title(&self) -> String {
        tr!("name")
    }

    fn app_id(&self) -> String {
        "reprise.charmap".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        repriseapp::start_forms().text.size(charmap::CLIENT)
    }

    fn event(&mut self, _win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        let mut reply = match &ev {
            Event::Key(k) => self.key(k, &ev, ctx),
            Event::Pointer(p) => self.pointer(p, &ev, ctx),
            Event::CloseRequest => Reply {
                quit: true,
                ..Reply::none()
            },
            Event::Resized | Event::Focus(_) => Reply::redraw(),
            _ => match form_input(&mut self.form, &ev, ctx) {
                Some(e) => self.form_event(e),
                None => Reply::none(),
            },
        };
        let size = ctx.forms().text.size(charmap::CLIENT);
        if ctx.size != size {
            reply.size = Some(size);
            reply.redraw = true;
        }
        reply
    }

    fn draw(&mut self, _win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        c.fill_rect(c.bounds(), forms.colors.button_face);
        let p = Paint {
            s: 1,
            ..forms.paint()
        };
        charmap::draw_window(
            c,
            &ctx.text,
            &self.glyphs,
            &p,
            &self.form,
            &self.map,
            self.grid_focus,
            forms.fonts.size_px,
        );
    }
}

impl Extended for CharMapWindow {
    fn fixed(&self) -> bool {
        true
    }

    fn access(&mut self, _win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let forms = ctx.forms();
        let focus = charmap::access_window(
            &self.form,
            &self.map,
            self.grid_focus,
            tree,
            ROOT,
            &ctx.text,
            forms.fonts.size_px,
            &forms.controls(),
        );
        if let Some(f) = focus {
            tree.set_focus(f);
        }
    }
}

/// Runs the program; `args` are the command line after the program's name.
pub fn main(_args: Vec<std::ffi::OsString>) {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    let fonts = reprisekit::text::families();
    let ui = repriseapp::start_forms().fonts.family;
    let font = ui
        .iter()
        .find_map(|f| fonts.iter().position(|g| g == f))
        .unwrap_or(0);
    repriseapp::run(Box::new(CharMapWindow::new(fonts, font))).unwrap_or_else(|e| e.exit());
}

#[cfg(test)]
mod tests;
