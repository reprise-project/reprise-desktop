//! About Reprise: the window around `reprisekit::controls::aboutbox`'s system view and `repriseapp::about::SystemFacts`;
//! it stays unmapped until the compositor's outputs are heard.

use repriseapp::about::{self, SystemFacts};
use repriseapp::keys::dialog_key;
use repriseapp::{App, BTN_LEFT, Caption, Ctx, Event, PointerInput, Reply, WinId};
use reprisekit::access::Tree;
use reprisekit::controls::aboutbox::{self, System};
use reprisekit::controls::form::{self, Form};
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;

struct AboutBox {
    facts: SystemFacts,
    logo: Option<Canvas>,
    /// Built with the first outputs, and again when they, the
    /// interface or the text size change.
    form: Option<Form>,
    /// Until the first `Event::Heads` the window has no surface: the
    /// display lines set its height.
    heard_outputs: bool,
}

impl AboutBox {
    fn system(&self) -> System {
        System {
            logo: self.logo.clone(),
            groups: self.facts.groups(),
        }
    }

    /// The form for the facts as they are and the interface and text size `ctx` draws in,
    /// rebuilt only when those are not the ones it shows.
    fn form(&mut self, ctx: &Ctx) -> &mut Form {
        let interface = about::interface_name(&ctx.theme);
        if self.facts.interface != interface {
            self.facts.interface = interface.into();
            self.form = None;
        }
        let forms = ctx.forms();
        let paint = forms.paint();
        if self.form.as_ref().is_some_and(|f| f.units() != paint.text) {
            self.form = None;
        }
        let px = ctx.theme.fonts.size_px;
        let system = self.system();
        self.form.get_or_insert_with(|| {
            let f = aboutbox::system_form(&system, &ctx.text, px, paint.text);
            f.drawn_with(&paint);
            f
        })
    }

    fn size(&self, ctx: &Ctx) -> (i32, i32) {
        ctx.forms().text.size(aboutbox::system_size(&self.system()))
    }

    fn answer(&self, ev: form::Event) -> Reply {
        match ev {
            form::Event::Default | form::Event::Cancel | form::Event::Pressed(aboutbox::OK) => {
                Reply {
                    quit: true,
                    ..Default::default()
                }
            }
            form::Event::Pressed(aboutbox::COPY) => Reply {
                copy: Some(self.facts.copy_text()),
                redraw: true,
                ..Default::default()
            },
            form::Event::None => Reply::none(),
            _ => Reply::redraw(),
        }
    }
}

impl App for AboutBox {
    fn title(&self) -> String {
        "About Reprise".into()
    }

    fn app_id(&self) -> String {
        "reprise.about".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        aboutbox::system_size(&self.system())
    }

    fn fixed(&self) -> bool {
        true
    }

    fn caption(&self, _win: WinId) -> Caption {
        Caption::Fixed
    }

    fn hidden(&self) -> bool {
        !self.heard_outputs
    }

    fn output_management(&self) -> bool {
        true
    }

    fn event(&mut self, _win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        match ev {
            Event::Heads(heads) => {
                if !self.heard_outputs {
                    let icons = &mut ctx.icons;
                    self.logo = self
                        .facts
                        .logo
                        .as_deref()
                        .and_then(|l| icons.get(l, aboutbox::LOGO))
                        .or_else(|| icons.get("computer", aboutbox::LOGO));
                }
                self.facts.displays = about::display_lines(&heads);
                self.heard_outputs = true;
                self.form = None;
                self.form(ctx);
                Reply {
                    size: Some(self.size(ctx)),
                    redraw: true,
                    ..Default::default()
                }
            }
            Event::CloseRequest => Reply {
                quit: true,
                ..Default::default()
            },
            Event::Key(k) => {
                let Some(key) = dialog_key(&k) else {
                    return Reply::none();
                };
                let ev = self.form(ctx).key(key);
                self.answer(ev)
            }
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                ..
            }) => {
                let px = ctx.forms().fonts.size_px;
                let hit = self.form(ctx).hit(x, y, &ctx.text, px);
                match self.form(ctx).click(hit) {
                    form::Event::None => Reply::none(),
                    _ => Reply::redraw(),
                }
            }
            Event::Pointer(PointerInput::Motion { x, y }) => {
                let px = ctx.forms().fonts.size_px;
                let hit = self.form(ctx).hit(x, y, &ctx.text, px);
                match self.form(ctx).motion(x, y, hit) {
                    form::Event::None => Reply::none(),
                    _ => Reply::redraw(),
                }
            }
            Event::Pointer(PointerInput::Leave) => match self.form(ctx).leave() {
                form::Event::None => Reply::none(),
                _ => Reply::redraw(),
            },
            Event::Pointer(PointerInput::Release {
                button: BTN_LEFT, ..
            }) => {
                let ev = self.form(ctx).release();
                self.answer(ev)
            }
            _ => Reply::none(),
        }
    }

    fn access(&mut self, _win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let px = ctx.forms().fonts.size_px;
        let form = self.form(ctx).clone();
        repriseapp::oneshot::access_form(&form, tree, &ctx.text, px);
    }

    fn draw(&mut self, _win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        let px = forms.fonts.size_px;
        c.fill_rect(c.bounds(), forms.colors.button_face);
        form::draw(
            c,
            &ctx.text,
            &Paint {
                s: 1,
                ..forms.paint()
            },
            self.form(ctx),
            true,
            px,
        );
    }
}

fn main() {
    env_logger::init();
    repriseapp::run(Box::new(AboutBox {
        facts: SystemFacts::read(),
        logo: None,
        form: None,
        heard_outputs: false,
    }));
}
