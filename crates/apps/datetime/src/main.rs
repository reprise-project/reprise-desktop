//! Date and Time Properties: the sheet behind a double click on the
//! taskbar clock. It reads and sets the clock, the time zone and network
//! time through systemd-timedated; polkit asks for a password when
//! needed. The sheet's date and time tick with the clock until edited.

mod sheet;
mod timedate;

use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use repriseapp::keys::sheet_input;
use repriseapp::oneshot::{self, MsgBox};
use repriseapp::{App, Child, Ctx, Event, Reply, WinId};
use reprisekit::access::Tree;
use reprisekit::controls::{msgbox, propsheet};
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use reprisekit::text::TextRenderer;
use reprisekit::theme::{FontConfig, Theme};

use sheet::{Local, Outcome, Sheet, System};

const ERROR_WIN: WinId = "error";
const TITLE: &str = "Date and Time Properties";

#[derive(Default)]
struct DateTimeApp {
    sheet: Option<Sheet>,
    system: System,
    zones: Vec<String>,
    /// An Apply under way on the worker thread, and whether an OK started
    /// it (the sheet closes when it succeeds).
    applying: Option<(Receiver<Result<System, String>>, bool)>,
    waker: Option<repriseapp::Waker>,
    error: Option<MsgBox>,
    /// The renderer the box is sized and drawn with: `children` has no
    /// `Ctx`, so it is loaded in `start`.
    dialog_text: Option<TextRenderer>,
    dialog_px: u32,
    quit: bool,
}

fn now() -> Local {
    reprisefs::locale::refresh_zone();
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let tm = reprisefs::locale::localtime(secs);
    Local {
        year: tm.tm_year + 1900,
        month: (tm.tm_mon + 1) as u32,
        day: tm.tm_mday as u32,
        h: tm.tm_hour as u32,
        m: tm.tm_min as u32,
        s: tm.tm_sec.min(59) as u32,
    }
}

fn start_ticker(waker: repriseapp::Waker) {
    std::thread::spawn(move || {
        loop {
            let into = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.subsec_nanos());
            std::thread::sleep(Duration::from_nanos(u64::from(1_000_000_000 - into)));
            waker.wake();
        }
    });
}

impl DateTimeApp {
    fn ensure(&mut self) -> &mut Sheet {
        let (system, zones) = (&self.system, &self.zones);
        self.sheet.get_or_insert_with(|| {
            Sheet::new(
                system,
                zones,
                now(),
                reprisefs::locale::first_weekday(),
                reprisefs::locale::is_12h(),
            )
        })
    }

    fn apply(&mut self, change: sheet::Change, close: bool) {
        if self.applying.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let _ = tx.send(timedate::apply(&change));
            if let Some(w) = waker {
                w.wake();
            }
        });
        self.applying = Some((rx, close));
    }

    fn wake(&mut self) -> Reply {
        if let Some((rx, close)) = &self.applying
            && let Ok(result) = rx.try_recv()
        {
            let close = *close;
            self.applying = None;
            match result {
                Ok(system) => {
                    self.system = system;
                    let t = now();
                    if let Some(s) = &mut self.sheet {
                        s.applied(&self.system, t);
                    }
                    if close {
                        self.quit = true;
                    }
                }
                Err(msg) => {
                    // The settings may be half changed: read what stuck.
                    if let Ok((system, _)) = timedate::read() {
                        self.system = system;
                    }
                    if let Some(s) = &mut self.sheet {
                        s.unapplied(&self.system);
                    }
                    self.error = Some(MsgBox::new(
                        TITLE,
                        msgbox::Fields {
                            text: format!("The clock could not be changed: {msg}"),
                            icon: None,
                            buttons: vec!["OK".into()],
                            default: 0,
                            min_text_h: None,
                        },
                    ));
                }
            }
        }
        if let Some(s) = &mut self.sheet {
            s.tick(now());
        }
        Reply {
            redraw: true,
            quit: self.quit,
            ..Reply::none()
        }
    }

    fn error_event(&mut self, ev: Event) -> Reply {
        let (Some(b), Some(text)) = (&mut self.error, &self.dialog_text) else {
            return Reply::none();
        };
        match b.event(&ev, text, self.dialog_px) {
            oneshot::Outcome::Closed(_) => {
                self.error = None;
                Reply::redraw_all()
            }
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::None => Reply::none(),
        }
    }
}

impl App for DateTimeApp {
    fn title(&self) -> String {
        TITLE.into()
    }

    fn app_id(&self) -> String {
        "reprise.datetime".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        sheet::CLIENT
    }

    fn fixed(&self) -> bool {
        true
    }

    fn children(&self) -> Vec<Child> {
        match (&self.error, &self.dialog_text) {
            (Some(b), Some(text)) => vec![b.child(ERROR_WIN, text, self.dialog_px)],
            _ => Vec::new(),
        }
    }

    fn start(&mut self, waker: repriseapp::Waker) {
        self.dialog_text = Some(TextRenderer::load(&FontConfig::default()));
        self.dialog_px = Theme::default().fonts.size_px;
        match timedate::read() {
            Ok((system, zones)) => {
                self.system = system;
                self.zones = zones;
            }
            Err(e) => {
                // Without timedated the sheet still shows the clock; an
                // Apply reports the missing service.
                log::error!("timedated does not answer: {e}");
                self.system.zone = std::fs::read_link("/etc/localtime")
                    .ok()
                    .and_then(|p| {
                        let s = p.to_string_lossy().into_owned();
                        s.split_once("zoneinfo/").map(|(_, z)| z.to_string())
                    })
                    .unwrap_or_else(|| "UTC".into());
            }
        }
        start_ticker(waker.clone());
        self.waker = Some(waker);
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        if matches!(ev, Event::Wake) {
            return self.wake();
        }
        if win == ERROR_WIN {
            return self.error_event(ev);
        }
        if matches!(ev, Event::CloseRequest) {
            return Reply {
                quit: true,
                ..Reply::none()
            };
        }
        let s = self.ensure();
        let Some(ev) = sheet_input(&mut s.sheet, &ev, ctx) else {
            return Reply::none();
        };
        match s.event(ev) {
            Outcome::None => Reply::none(),
            Outcome::Redraw => Reply::redraw(),
            Outcome::Apply(change) => {
                self.apply(change, false);
                Reply::redraw()
            }
            Outcome::ApplyAndClose(change) => {
                self.apply(change, true);
                Reply::redraw()
            }
            Outcome::Close => Reply {
                quit: true,
                ..Reply::none()
            },
        }
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        if win == ERROR_WIN {
            if let (Some(b), Some(text)) = (&self.error, &self.dialog_text) {
                b.access(tree, text, self.dialog_px);
            }
        } else if let Some(s) = &self.sheet {
            oneshot::access_sheet(&s.sheet, tree, &ctx.text, ctx.theme.fonts.size_px);
        }
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let cs = &ctx.theme.colors;
        if win == ERROR_WIN {
            if let (Some(b), Some(text)) = (&self.error, &self.dialog_text) {
                b.draw(c, &ctx.theme, text, self.dialog_px);
            }
            return;
        }
        let s = self.ensure();
        c.fill_rect(c.bounds(), cs.button_face);
        propsheet::draw(
            c,
            &ctx.text,
            &Paint {
                s: 1,
                ..ctx.theme.paint()
            },
            &s.sheet,
            true,
            ctx.theme.fonts.size_px,
        );
    }
}

fn main() {
    env_logger::init();
    repriseapp::run(Box::new(DateTimeApp::default()));
}
