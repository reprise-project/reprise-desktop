//! Drag and drop between programs through repriseapp: the left tile drags the files named on the
//! command line, the right area takes drops and prints every drag event to stderr.
//!
//! `cargo run -p repriseapp --example dnd -- FILE...`

use repriseapp::{
    Accept, Actions, App, BTN_LEFT, BTN_RIGHT, Ctx, DragAction, DragOut, DropKind, Dropped, Event,
    Offered, PointerInput, Reply, WinId,
};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::Align;
use reprisekit::theme::Color;
use std::path::PathBuf;

const W: i32 = 480;
const H: i32 = 240;
const THRESHOLD: i32 = 4;

fn tile() -> Rect {
    Rect::new(20, 20, 160, 200)
}

fn area() -> Rect {
    Rect::new(200, 20, 260, 200)
}

struct Demo {
    paths: Vec<PathBuf>,
    pressed: Option<(i32, i32)>,
    dragging: bool,
    hover: bool,
    lines: Vec<String>,
    asking: bool,
    offered: Offered,
}

impl Demo {
    fn say(&mut self, line: String) {
        eprintln!("dnd: {line}");
        self.lines.push(line);
        let n = self.lines.len();
        if n > 12 {
            self.lines.drain(..n - 12);
        }
    }

    fn image(&self, ctx: &Ctx) -> Canvas {
        let mut c = Canvas::new(48, 48);
        c.fill_rect(Rect::new(0, 0, 48, 48), ctx.theme.colors.hilight);
        ctx.text.draw(
            &mut c,
            Rect::new(0, 0, 48, 48),
            &self.paths.len().to_string(),
            true,
            ctx.theme.colors.hilight_text,
            Align::Center,
            ctx.theme.fonts.size_px,
        );
        c
    }

    fn answer(&mut self, x: i32, y: i32) -> Reply {
        let o = self.offered;
        let kind = if o.link {
            Some(DropKind::Uris)
        } else if o.files {
            Some(DropKind::Files)
        } else if o.text {
            Some(DropKind::Text)
        } else {
            None
        };
        let accept = kind.filter(|_| area().contains(x, y)).map(|kind| Accept {
            kind,
            actions: Actions::ALL,
            preferred: Some(DragAction::Copy),
        });
        let hover = accept.is_some();
        let redraw = hover != self.hover;
        self.hover = hover;
        Reply {
            redraw,
            ..Reply::drag_accept(accept)
        }
    }
}

impl App for Demo {
    fn title(&self) -> String {
        "Drag and Drop".into()
    }
    fn app_id(&self) -> String {
        "reprise.dnd".into()
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
            Event::Pointer(PointerInput::Press { x, y, button, .. })
                if (button == BTN_LEFT || button == BTN_RIGHT) && tile().contains(x, y) =>
            {
                self.pressed = Some((x, y));
                Reply::none()
            }
            Event::Pointer(PointerInput::Release { .. }) => {
                self.pressed = None;
                Reply::none()
            }
            Event::Pointer(PointerInput::Motion { x, y }) => match self.pressed {
                Some((px, py))
                    if !self.dragging
                        && !self.paths.is_empty()
                        && ((x - px).abs() > THRESHOLD || (y - py).abs() > THRESHOLD) =>
                {
                    self.pressed = None;
                    self.dragging = true;
                    self.say(format!("start drag of {} file(s)", self.paths.len()));
                    Reply::start_drag(DragOut {
                        paths: self.paths.clone(),
                        image: self.image(ctx),
                        grab: (24, 24),
                    })
                }
                _ => Reply::none(),
            },
            Event::DragEnter {
                x,
                y,
                offered,
                source,
                action,
                own,
            } => {
                self.say(format!("enter {x},{y} {offered:?} {action:?} own={own}"));
                eprintln!("dnd: source allows {source:?}");
                self.offered = offered;
                self.answer(x, y)
            }
            Event::DragMotion { x, y, action } => {
                if let Some(a) = action {
                    eprintln!("dnd: motion {x},{y} {a:?}");
                }
                self.answer(x, y)
            }
            Event::DragFiles { paths } => {
                self.say(format!("carries {} file(s)", paths.len()));
                Reply::redraw()
            }
            Event::DragLeave => {
                self.hover = false;
                self.say("leave".into());
                Reply::redraw()
            }
            Event::DragDrop {
                data,
                action,
                source,
                ..
            } => {
                self.hover = false;
                match &data {
                    Dropped::Files(p) => {
                        self.say(format!("drop {action:?}: {} file(s)", p.len()));
                        for p in p {
                            self.say(format!("  {}", p.display()));
                        }
                    }
                    Dropped::Uris(u) => {
                        self.say(format!("drop {action:?}: {} item(s)", u.len()));
                        for u in u {
                            self.say(format!("  {u:?}"));
                        }
                    }
                    Dropped::Text(t) => self.say(format!("drop {action:?}: text {t:?}")),
                }
                if action == DragAction::Ask {
                    self.asking = true;
                    self.say(format!(
                        "ask: C copy{}, M move{}, Esc cancel",
                        if source.copy { "" } else { " (not allowed)" },
                        if source.move_ { "" } else { " (not allowed)" },
                    ));
                }
                Reply::redraw()
            }
            Event::DropCancelled => {
                self.asking = false;
                self.say("drop cancelled".into());
                Reply::redraw()
            }
            Event::DragEnd { dropped, action } => {
                self.dragging = false;
                self.say(format!("end dropped={dropped} action={action:?}"));
                Reply::redraw()
            }
            Event::Key(k) if self.asking => {
                let answer = match k.utf8.as_deref() {
                    Some("c") => Some(DragAction::Copy),
                    Some("m") => Some(DragAction::Move),
                    _ if k.sym == 0xff1b => None,
                    _ => return Reply::none(),
                };
                self.asking = false;
                self.say(format!("answer {answer:?}"));
                Reply {
                    redraw: true,
                    ..Reply::answer_drop(answer)
                }
            }
            _ => Reply::none(),
        }
    }

    fn draw(&mut self, _win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let cs = &ctx.theme.colors;
        let px = ctx.theme.fonts.size_px;
        c.fill_rect(Rect::new(0, 0, ctx.size.0, ctx.size.1), cs.button_face);
        c.fill_rect(tile(), cs.hilight);
        let t = tile();
        ctx.text.draw(
            c,
            Rect::new(t.x, t.y, t.width, 20),
            "Drag me",
            true,
            cs.hilight_text,
            Align::Center,
            px,
        );
        for (i, p) in self.paths.iter().enumerate() {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
            ctx.text.draw(
                c,
                Rect::new(t.x + 4, t.y + 24 + 16 * i as i32, t.width - 8, 16),
                &name.unwrap_or_default(),
                false,
                cs.hilight_text,
                Align::Left,
                px,
            );
        }
        let a = area();
        let bg = if self.hover {
            Color::rgb(200, 230, 200)
        } else {
            cs.window
        };
        c.fill_rect(a, bg);
        for (i, line) in self.lines.iter().enumerate() {
            ctx.text.draw(
                c,
                Rect::new(a.x + 4, a.y + 4 + 16 * i as i32, a.width - 8, 16),
                line,
                false,
                cs.window_text,
                Align::Left,
                px,
            );
        }
    }
}

fn main() {
    env_logger::init();
    let paths = std::env::args_os().skip(1).map(PathBuf::from).collect();
    repriseapp::run(Box::new(Demo {
        paths,
        pressed: None,
        dragging: false,
        hover: false,
        lines: Vec::new(),
        asking: false,
        offered: Offered::default(),
    }));
}
