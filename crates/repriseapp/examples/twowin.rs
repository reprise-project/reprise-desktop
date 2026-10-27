//! Two windows: a main one with a button that opens a fixed 300x150
//! dialog with OK and Cancel, for checking the compositor's dialog frame:
//! the close box alone, centred on its owner, no taskbar button, no
//! resize band.
//!
//! `cargo run -p repriseapp --example twowin`

use repriseapp::{App, BTN_LEFT, Child, Ctx, Event, MAIN, PointerInput, Reply, WinId};
use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W};
use reprisekit::look::Paint;
use reprisekit::look::widgets::draw_button;
use reprisekit::pointer::Interaction;
use reprisekit::raster::{Canvas, Rect};

const DIALOG: WinId = WinId::named("dialog");

struct TwoWin {
    open: bool,
    modal: bool,
}

/// The main window's "Dialog..." button.
fn open_button() -> Rect {
    Rect::new(20, 20, BUTTON_W, BUTTON_H)
}

/// OK and Cancel along the dialog's bottom right.
fn dialog_buttons() -> (Rect, Rect) {
    let y = 150 - 10 - BUTTON_H;
    let cancel = Rect::new(300 - 10 - BUTTON_W, y, BUTTON_W, BUTTON_H);
    let ok = Rect::new(cancel.x - 6 - BUTTON_W, y, BUTTON_W, BUTTON_H);
    (ok, cancel)
}

impl App for TwoWin {
    fn title(&self) -> String {
        "Two Windows".into()
    }
    fn app_id(&self) -> String {
        "reprise.twowin".into()
    }
    fn initial_size(&self) -> (i32, i32) {
        (480, 320)
    }

    fn children(&self) -> Vec<Child> {
        if !self.open {
            return Vec::new();
        }
        vec![Child {
            id: DIALOG,
            title: "Dialog".into(),
            size: (300, 150),
            fixed: true,
            modal: self.modal,
        }]
    }

    fn event(&mut self, win: WinId, ev: Event, _ctx: &mut Ctx) -> Reply {
        match (win, ev) {
            (MAIN, Event::CloseRequest) => Reply {
                quit: true,
                ..Default::default()
            },
            (DIALOG, Event::CloseRequest) => {
                self.open = false;
                Reply::redraw_all()
            }
            (
                MAIN,
                Event::Pointer(PointerInput::Press {
                    x,
                    y,
                    button: BTN_LEFT,
                    ..
                }),
            ) if open_button().contains(x, y) => {
                self.open = true;
                Reply::redraw()
            }
            (
                DIALOG,
                Event::Pointer(PointerInput::Press {
                    x,
                    y,
                    button: BTN_LEFT,
                    ..
                }),
            ) => {
                let (ok, cancel) = dialog_buttons();
                if ok.contains(x, y) || cancel.contains(x, y) {
                    self.open = false;
                    return Reply::redraw_all();
                }
                Reply::none()
            }
            _ => Reply::none(),
        }
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let cs = &ctx.theme.colors;
        c.fill_rect(Rect::new(0, 0, ctx.size.0, ctx.size.1), cs.button_face);
        let px = ctx.theme.fonts.size_px;
        if win == MAIN {
            draw_button(
                c,
                &ctx.text,
                &Paint {
                    s: 1,
                    ..ctx.theme.paint()
                },
                open_button(),
                "&Dialog...",
                false,
                Interaction::Idle,
                true,
                false,
                px,
            );
        } else {
            let (ok, cancel) = dialog_buttons();
            draw_button(
                c,
                &ctx.text,
                &Paint {
                    s: 1,
                    ..ctx.theme.paint()
                },
                ok,
                "OK",
                true,
                Interaction::Idle,
                true,
                false,
                px,
            );
            draw_button(
                c,
                &ctx.text,
                &Paint {
                    s: 1,
                    ..ctx.theme.paint()
                },
                cancel,
                "Cancel",
                false,
                Interaction::Idle,
                true,
                false,
                px,
            );
        }
    }
}

impl repriseapp::Extended for TwoWin {}

fn main() {
    env_logger::init();
    // `--modal` makes the dialog gate the main window's input.
    let modal = std::env::args().any(|a| a == "--modal");
    repriseapp::run(Box::new(TwoWin { open: false, modal })).unwrap_or_else(|e| e.exit());
}
