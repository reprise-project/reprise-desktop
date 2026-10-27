//! The smallest program: one window holding a form with a label and a Close button.
//!
//! `cargo run -p repriseapp --example hello`
use repriseapp::keys::form_input;
use repriseapp::{App, Ctx, Event, Extended, Reply, WinId};
use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W};
use reprisekit::controls::form::{self, Form, Item};
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};

const SIZE: (i32, i32) = (240, 100);

struct Hello {
    form: Form,
}

impl Hello {
    fn new() -> Hello {
        let close = Rect::new(
            SIZE.0 - 12 - BUTTON_W,
            SIZE.1 - 12 - BUTTON_H,
            BUTTON_W,
            BUTTON_H,
        );
        let form = Form::new(vec![
            Item::label(
                "text",
                Rect::new(12, 16, SIZE.0 - 24, 16),
                "Hello from Reprise.",
            ),
            Item::button("close", close, "Close", true, true),
        ]);
        Hello { form }
    }
}

impl App for Hello {
    fn title(&self) -> String {
        "Hello".into()
    }

    fn app_id(&self) -> String {
        "reprise.hello".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        SIZE
    }

    fn event(&mut self, _win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        let quit = Reply {
            quit: true,
            ..Reply::none()
        };
        if ev == Event::CloseRequest {
            return quit;
        }
        match form_input(&mut self.form, &ev, ctx) {
            Some(form::Event::Pressed("close") | form::Event::Default | form::Event::Cancel) => {
                quit
            }
            Some(form::Event::None) | None => Reply::none(),
            Some(_) => Reply::redraw(),
        }
    }

    fn draw(&mut self, _win: WinId, c: &mut Canvas, ctx: &Ctx) {
        c.fill_rect(c.bounds(), ctx.theme.colors.button_face);
        let p = Paint {
            s: 1,
            ..ctx.theme.paint()
        };
        form::draw(c, &ctx.text, &p, &self.form, true, ctx.theme.fonts.size_px);
    }
}

impl Extended for Hello {}

fn main() {
    env_logger::init();
    repriseapp::run(Box::new(Hello::new())).unwrap_or_else(|e| e.exit());
}
