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
    /// The client size last asked for, at the text size of the last event.
    asked: (i32, i32),
}

impl AboutBox {
    fn new(facts: SystemFacts) -> Self {
        let mut about = AboutBox {
            facts,
            logo: None,
            form: None,
            heard_outputs: false,
            asked: (0, 0),
        };
        about.asked = aboutbox::system_size(&about.system());
        about
    }

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

    fn dispatch(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
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
                Reply::redraw()
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
}

impl App for AboutBox {
    fn title(&self) -> String {
        "About Reprise".into()
    }

    fn app_id(&self) -> String {
        "reprise.about".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        self.asked
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
        let mut reply = self.dispatch(ev, ctx);
        let size = self.size(ctx);
        if size != self.asked {
            self.asked = size;
            reply.size = Some(size);
            reply.redraw = true;
        }
        reply
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
    repriseapp::run(Box::new(AboutBox::new(SystemFacts::read())));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The system view fits its text under every interface and text size: "Operating system:"
    /// in its label column, and long processor and graphics names cut to their grown column.
    #[test]
    fn the_system_view_fits_its_text() {
        let facts = SystemFacts {
            version: "0.1.0".into(),
            interface: "Classic".into(),
            os: "Arch Linux".into(),
            kernel: "6.17.2-arch1-1".into(),
            arch: "x86_64".into(),
            hostname: "desk".into(),
            model: Some("ASUS ROG Strix G16 G614JV_G614JV 1.0".into()),
            cpu: Some("13th Gen Intel(R) Core(TM) i7-13650HX".into()),
            cores: Some(14),
            threads: 20,
            mhz: Some(4900.0),
            gpus: vec!["NVIDIA GeForce RTX 4060 Max-Q / Mobile (AD107M)".into()],
            ram: Some(16 << 30),
            ram_free: Some(9 << 30),
            disk: Some((1 << 40, 400 << 30)),
            user: "Jo Doe".into(),
            login: "jo".into(),
            uptime: Some(11_520),
            displays: vec![
                "2560 x 1600, 240 Hz (Built-in display)".into(),
                "3840 x 2160, 60 Hz, 150% (DELL U2720Q)".into(),
            ],
            ..SystemFacts::default()
        };
        let system = System {
            logo: None,
            groups: facts.groups(),
        };
        let over = reprisekit::interfaces::form_overflows(|_, theme, text| {
            let px = theme.text.base_px(theme.fonts.size_px);
            vec![(
                "system".into(),
                aboutbox::system_form(&system, text, px, theme.text),
            )]
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    fn test_ctx(text_size: u32) -> Ctx {
        Ctx {
            theme: reprisekit::theme::Theme::default(),
            text: reprisekit::text::TextRenderer::load(&reprisekit::theme::FontConfig::default()),
            icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
                "test",
                &[],
            )),
            caret_on: true,
            size: (300, 300),
            maximized: false,
            origin: (0, 0),
            wake: repriseapp::Waker::noop(),
            heads: Default::default(),
            want_tick: Default::default(),
            text_size,
        }
    }

    #[test]
    fn the_window_opens_and_stays_at_the_text_size() {
        let mut about = AboutBox::new(SystemFacts::default());
        let mut ctx = test_ctx(150);
        about.event(repriseapp::MAIN, Event::Heads(Vec::new()), &mut ctx);
        let system = aboutbox::system_size(&about.system());
        let at = |size| test_ctx(size).forms().text.size(system);
        assert_ne!(at(150), at(100));
        assert_eq!(about.initial_size(), at(150), "the window maps at 150%");

        ctx.text_size = 200;
        let reply = about.event(repriseapp::MAIN, Event::Resized, &mut ctx);
        assert_eq!(reply.size, Some(at(200)), "a live change to 200% grows it");
        assert_eq!(about.initial_size(), at(200));
    }
}
