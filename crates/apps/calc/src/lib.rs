//! Calculator: the window around `reprisekit::controls::calculator`, with its menu bar and
//! the clipboard.

#[cfg(test)]
reprisebus::test_session!();

reprisekit::strings!("calc");

use repriseapp::keys::{AltArm, form_input};
use repriseapp::{App, Ctx, Event, Extended, KeyInput, PointerInput, Reply, WinId};
use reprisekit::access::{ROOT, Tree};
use reprisekit::controls::calculator::{self, Calc, Mode, Press};
use reprisekit::controls::form::{self, Form};
use reprisekit::controls::hosted::Hosted;
use reprisekit::controls::menu::MenuItem;
use reprisekit::controls::menubar::{self, Action, Bar, Key};
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::textedit::{EditKey, keysym};
use reprisekit::tr;

mod title {
    pub const EDIT: usize = 0;
    pub const VIEW: usize = 1;
}

mod row {
    pub const COPY: usize = 0;
    pub const PASTE: usize = 1;
    pub const STANDARD: usize = 0;
    pub const SCIENTIFIC: usize = 1;
    pub const HISTORY: usize = 3;
}

fn menus() -> Vec<(String, Vec<MenuItem>)> {
    let mi = |label: String, shortcut: Option<&str>| {
        MenuItem::item(&label).shortcut(shortcut.map(str::to_string))
    };
    vec![
        (
            tr!("menu-edit"),
            vec![
                mi(tr!("menu-copy"), Some("Ctrl+C")),
                mi(tr!("menu-paste"), Some("Ctrl+V")),
            ],
        ),
        (
            tr!("menu-view"),
            vec![
                mi(tr!("menu-standard"), None).radio(true),
                mi(tr!("menu-scientific"), None).radio(true),
                MenuItem::Separator,
                mi(tr!("menu-history"), Some("Ctrl+H")),
            ],
        ),
    ]
}

struct Calculator {
    calc: Calc,
    history: bool,
    form: Form,
    bar: Bar,
    alt_arm: AltArm,
    /// The client size last asked for.
    asked: (i32, i32),
}

impl Calculator {
    fn new() -> Calculator {
        let calc = Calc::new(Mode::Standard);
        let mut c = Calculator {
            form: calculator::form(&calc, false),
            calc,
            history: false,
            bar: Bar::new(menus()),
            alt_arm: AltArm::default(),
            asked: (0, 0),
        };
        c.sync_menus();
        c
    }

    fn sync_menus(&mut self) {
        let set = |item: &mut MenuItem, on: bool| {
            if let MenuItem::Entry { checked, .. } = item {
                *checked = on;
            }
        };
        let view = &mut self.bar.menus[title::VIEW];
        set(&mut view[row::STANDARD], self.calc.mode == Mode::Standard);
        set(
            &mut view[row::SCIENTIFIC],
            self.calc.mode == Mode::Scientific,
        );
        set(&mut view[row::HISTORY], self.history);
    }

    /// The client size for the mode and history at `ctx`'s text size, under the menu bar.
    fn size(&self, ctx: &Ctx) -> (i32, i32) {
        let (w, h) = ctx
            .forms()
            .text
            .size(calculator::client(self.calc.mode, self.history));
        (w, h + menubar::bar_h(&ctx.theme.controls()))
    }

    fn bar_layout(&self, ctx: &Ctx) -> menubar::Layout {
        self.bar
            .layout(
                &ctx.text,
                ctx.theme.fonts.size_px,
                Rect::new(0, 0, ctx.size.0, ctx.size.1),
                &ctx.theme.controls(),
            )
            .reaching(ctx.menu_within())
    }

    /// Rebuilds the pad for a new mode or history panel.
    fn relayout(&mut self) {
        let origin = self.form.origin;
        self.form = calculator::form(&self.calc, self.history);
        self.form.origin = origin;
        self.sync_menus();
    }

    fn input(&mut self, input: calculator::Input) -> Reply {
        if self.calc.input(input) {
            calculator::follow(&mut self.form, &self.calc);
            Reply::redraw()
        } else {
            Reply::none()
        }
    }

    fn command(&mut self, title: usize, row: usize) -> Reply {
        match (title, row) {
            (title::EDIT, row::COPY) => Reply {
                copy: self.calc.copy_text(),
                ..Reply::none()
            },
            (title::EDIT, row::PASTE) => Reply {
                paste: true,
                ..Reply::redraw()
            },
            (title::VIEW, row::STANDARD) => self.set_mode(Mode::Standard),
            (title::VIEW, row::SCIENTIFIC) => self.set_mode(Mode::Scientific),
            (title::VIEW, row::HISTORY) => {
                self.history = !self.history;
                self.relayout();
                Reply::redraw()
            }
            _ => Reply::redraw(),
        }
    }

    fn set_mode(&mut self, mode: Mode) -> Reply {
        self.calc.set_mode(mode);
        self.relayout();
        Reply::redraw()
    }

    fn press(&mut self, id: &str) -> Reply {
        match calculator::press_of(id) {
            Some(Press::Input(i)) => self.input(i),
            Some(Press::Angle(a)) => {
                self.calc.angle = a;
                calculator::follow(&mut self.form, &self.calc);
                Reply::redraw()
            }
            Some(Press::ClearHistory) => {
                self.calc.clear_history();
                calculator::follow(&mut self.form, &self.calc);
                Reply::redraw()
            }
            None => Reply::redraw(),
        }
    }

    fn form_event(&mut self, ev: form::Event) -> Reply {
        match ev {
            form::Event::Pressed(id) => self.press(id),
            form::Event::Changed(calculator::HISTORY_LIST) => {
                if let Some(i) = calculator::picked_line(&self.form, &self.calc) {
                    self.calc.reuse(i);
                }
                Reply::redraw()
            }
            form::Event::Changed(id) => self.press(id),
            form::Event::None => Reply::none(),
            _ => Reply::redraw(),
        }
    }

    fn key(&mut self, k: &KeyInput, ev: &Event, ctx: &Ctx) -> Reply {
        if k.sym == keysym::ALT_L || k.sym == keysym::ALT_R {
            self.alt_arm.press_alt();
            return Reply::none();
        }
        self.alt_arm.interrupt();
        let lay = self.bar_layout(ctx);
        let px = ctx.theme.fonts.size_px;
        let bar_key = match k.sym {
            keysym::F10 => Some(Key::Activate),
            _ if !self.bar.is_active() => None,
            keysym::ESCAPE => Some(Key::Escape),
            keysym::LEFT | keysym::KP_LEFT => Some(Key::Left),
            keysym::RIGHT | keysym::KP_RIGHT => Some(Key::Right),
            keysym::UP | keysym::KP_UP => Some(Key::Up),
            keysym::DOWN | keysym::KP_DOWN => Some(Key::Down),
            keysym::RETURN | keysym::KP_ENTER => Some(Key::Enter),
            _ => None,
        };
        let access = k.utf8.as_deref().and_then(|s| s.chars().next());
        let bar_key = bar_key.or_else(|| {
            access
                .filter(|c| c.is_alphanumeric() && (k.mods.alt || self.bar.is_active()))
                .map(|c| Key::Char(c, k.mods.alt))
        });
        if let Some(bk) = bar_key {
            match self.bar.key(bk, &lay, &ctx.text, &ctx.theme, px) {
                Action::Chosen { title, row, .. } => return self.command(title, row),
                Action::Redraw => return Reply::redraw(),
                Action::Ignored => {}
            }
        }
        match &k.edit {
            Some(EditKey::Copy) => return self.command(title::EDIT, row::COPY),
            Some(EditKey::Paste) => return self.command(title::EDIT, row::PASTE),
            _ => {}
        }
        if k.mods.ctrl && matches!(k.sym, 0x68 | 0x48) {
            return self.command(title::VIEW, row::HISTORY);
        }
        if !k.mods.alt
            && let Some(input) = calculator::input_for(k.sym, access, k.mods.ctrl)
        {
            return self.input(input);
        }
        match form_input(&mut self.form, ev, ctx) {
            Some(e) => self.form_event(e),
            None => Reply::none(),
        }
    }

    fn key_up(&mut self, k: &KeyInput, ctx: &Ctx) -> Reply {
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

    fn pointer(&mut self, p: &PointerInput, ev: &Event, ctx: &Ctx) -> Reply {
        if matches!(p, PointerInput::Press { .. }) {
            self.alt_arm.interrupt();
        }
        let lay = self.bar_layout(ctx);
        let px = ctx.theme.fonts.size_px;
        let mut title_lit = false;
        match *p {
            PointerInput::Press { x, y, .. } => {
                match self.bar.press(x, y, &lay, &ctx.text, &ctx.theme, px) {
                    Action::Chosen { title, row, .. } => return self.command(title, row),
                    Action::Redraw => return Reply::redraw(),
                    Action::Ignored => {}
                }
            }
            PointerInput::Motion { x, y } if self.bar.open.is_some() => {
                self.bar.motion(x, y, &lay, &ctx.text, &ctx.theme, px);
                return Reply::redraw();
            }
            PointerInput::Motion { x, y } => title_lit = self.bar.hover(x, y, &lay),
            PointerInput::Leave => title_lit = self.bar.leave(),
            _ => {}
        }
        let mut reply = match form_input(&mut self.form, ev, ctx) {
            Some(e) => self.form_event(e),
            None => Reply::none(),
        };
        reply.redraw |= title_lit && ctx.styled();
        reply
    }
}

impl App for Calculator {
    fn title(&self) -> String {
        tr!("name")
    }

    fn app_id(&self) -> String {
        "reprise.calc".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        self.asked
    }

    fn event(&mut self, _win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.form.origin = (0, menubar::bar_h(&ctx.theme.controls()));
        let mut reply = match &ev {
            Event::Key(k) => self.key(k, &ev, ctx),
            Event::KeyUp(k) => self.key_up(k, ctx),
            Event::Pointer(p) => self.pointer(p, &ev, ctx),
            Event::Pasted(text) => {
                if self.calc.paste(text) {
                    calculator::follow(&mut self.form, &self.calc);
                }
                Reply::redraw()
            }
            Event::CloseRequest => Reply {
                quit: true,
                ..Reply::none()
            },
            Event::Focus(false) => {
                self.alt_arm.interrupt();
                self.bar.close();
                Reply::redraw()
            }
            Event::Resized | Event::Focus(_) => Reply::redraw(),
            _ => match form_input(&mut self.form, &ev, ctx) {
                Some(e) => self.form_event(e),
                None => Reply::none(),
            },
        };
        let size = self.size(ctx);
        if size != self.asked {
            self.asked = size;
            reply.size = Some(size);
            reply.redraw = true;
        }
        reply
    }

    fn draw(&mut self, _win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        self.form.origin = (0, menubar::bar_h(&ctx.theme.controls()));
        c.fill_rect(c.bounds(), forms.colors.button_face);
        let p = Paint {
            s: 1,
            ..forms.paint()
        };
        calculator::draw(
            c,
            &ctx.text,
            &p,
            &self.form,
            &self.calc,
            forms.fonts.size_px,
        );
        let lay = self.bar_layout(ctx);
        self.bar.draw(c, &ctx.text, &ctx.theme, &lay);
    }
}

impl Extended for Calculator {
    fn fixed(&self) -> bool {
        true
    }

    fn text_sized(&self) -> bool {
        true
    }

    fn popup(&mut self, _win: WinId, _ctx: &Ctx) -> Option<Box<dyn Hosted + '_>> {
        self.bar
            .open
            .is_some()
            .then(|| Box::new(&mut self.bar) as Box<dyn Hosted>)
    }

    fn access(&mut self, _win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let menus = self.bar.access(&self.bar_layout(ctx), tree, ROOT);
        let px = ctx.forms().fonts.size_px;
        let focus = calculator::access(&self.form, &self.calc, tree, ROOT, &ctx.text, px);
        if let Some(f) = menus.or(focus) {
            tree.set_focus(f);
        }
    }
}

/// Runs the program; `args` are the command line after the program's name.
pub fn main(_args: Vec<std::ffi::OsString>) {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    let mut calc = Calculator::new();
    let forms = repriseapp::start_forms();
    calc.asked = {
        let (w, h) = forms.text.size(calculator::client(Mode::Standard, false));
        (w, h + menubar::bar_h(&forms.controls()))
    };
    repriseapp::run(Box::new(calc)).unwrap_or_else(|e| e.exit());
}

#[cfg(test)]
mod tests;
