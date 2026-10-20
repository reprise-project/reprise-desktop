//! `cargo run -p repriseapp --example controls`

use repriseapp::keys::dialog_key;
use repriseapp::{App, BTN_LEFT, Ctx, Event, PointerInput, Reply, WinId};
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::statusbar::{self, Panel, StatusBar};
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::Align;
use reprisekit::textedit::TextEdit;
use reprisekit::trackbar::{self, TrackBar};

const W: i32 = 420;
const H: i32 = 300;

struct Controls {
    form: Form,
    bar: StatusBar,
}

fn build_form() -> Form {
    let mut greyed = TrackBar::new(13, 18);
    greyed.enabled = false;
    Form::new(vec![
        Item::new(
            "lbl",
            Rect::new(12, 14, 70, 13),
            Control::Label {
                text: "Fi&nd what:".into(),
                enabled: true,
                align: Align::Left,
            },
        ),
        Item::new(
            "find",
            Rect::new(90, 11, 200, 20),
            Control::Edit {
                edit: TextEdit::new("", false),
                first: std::cell::Cell::new(0),
                enabled: true,
            },
        ),
        Item::new(
            "grp",
            Rect::new(12, 48, 180, 74),
            Control::Group("Direction".into()),
        ),
        Item::radio("up", 24, 68, "&Up", "dir"),
        Item::radio("down", 24, 90, "&Down", "dir").checked(true),
        Item::radio("off", 212, 68, "Disa&bled", "other").enabled(false),
        Item::new(
            "chk",
            Rect::new(212, 90, 13, 13),
            Control::Check {
                label: "Disabled check".into(),
                checked: true,
                enabled: false,
                mixed: None,
            },
        ),
        Item::new(
            "track",
            Rect::new(24, 140, 71, trackbar::HEIGHT),
            Control::TrackBar(greyed),
        ),
        Item::edit("greyed", Rect::new(212, 116, 150, 20), "Greyed text", false),
        Item::combo("list", 212, 146, 150, &["(None)", "Blank"], 4),
        Item::check("box", 24, 196, "&Check box"),
        Item::button("ok", Rect::new(236, 236, 75, 23), "OK", true, true),
        Item::button("cancel", Rect::new(321, 236, 75, 23), "Cancel", true, false),
    ])
}

fn build_bar() -> StatusBar {
    StatusBar {
        panels: vec![
            Panel {
                text: String::new(),
                width: 0,
                pad: statusbar::TEXT_PAD,
                icon: None,
            },
            Panel {
                text: "Ln 1, Col 1".into(),
                width: 356,
                pad: statusbar::TEXT_PAD,
                icon: None,
            },
        ],
        grip: true,
    }
}

fn reply_for(out: form::Event) -> Reply {
    if out == form::Event::None {
        Reply::none()
    } else {
        Reply::redraw()
    }
}

impl App for Controls {
    fn title(&self) -> String {
        "Controls".into()
    }
    fn app_id(&self) -> String {
        "reprise.controls".into()
    }
    fn initial_size(&self) -> (i32, i32) {
        (W, H)
    }

    fn event(&mut self, _win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        match ev {
            Event::CloseRequest => Reply {
                quit: true,
                ..Default::default()
            },
            Event::Blink => {
                self.form.blink(ctx.caret_on);
                Reply::redraw()
            }
            Event::Key(k) => {
                let Some(key) = dialog_key(&k) else {
                    return Reply::none();
                };
                reply_for(self.form.key(key))
            }
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                ..
            }) => {
                let px = ctx.theme.fonts.size_px;
                let hit = self.form.hit(x, y, &ctx.text, px);
                reply_for(self.form.click(hit))
            }
            Event::Pointer(PointerInput::Release {
                button: BTN_LEFT, ..
            }) => reply_for(self.form.release()),
            Event::Pointer(PointerInput::Motion { x, y }) => {
                let px = ctx.theme.fonts.size_px;
                let hit = self.form.hit(x, y, &ctx.text, px);
                reply_for(self.form.motion(x, y, hit))
            }
            Event::Pointer(PointerInput::Leave) => reply_for(self.form.leave()),
            Event::Resized => Reply::redraw(),
            _ => Reply::none(),
        }
    }

    fn draw(&mut self, _win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let cs = &ctx.theme.colors;
        let px = ctx.theme.fonts.size_px;
        c.fill_rect(Rect::new(0, 0, ctx.size.0, ctx.size.1), cs.button_face);
        form::draw(
            c,
            &ctx.text,
            &Paint {
                s: 1,
                ..ctx.theme.paint()
            },
            &self.form,
            true,
            px,
        );
        let r = Rect::new(
            0,
            ctx.size.1 - statusbar::HEIGHT,
            ctx.size.0,
            statusbar::HEIGHT,
        );
        statusbar::draw(
            c,
            &ctx.text,
            &Paint {
                s: 1,
                ..ctx.theme.paint()
            },
            &self.bar,
            r,
            px,
        );
    }
}

fn main() {
    env_logger::init();
    repriseapp::run(Box::new(Controls {
        form: build_form(),
        bar: build_bar(),
    }));
}
