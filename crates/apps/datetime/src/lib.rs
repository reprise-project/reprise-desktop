//! Date and Time Properties: the sheet behind a double click on the
//! taskbar clock. It reads and sets the clock, the time zone and network
//! time through systemd-timedated; polkit asks for a password when
//! needed. The sheet's date and time tick with the clock until edited.

#[cfg(test)]
reprisebus::test_session!();

reprisekit::strings!("datetime");

mod sheet;
mod timedate;

use std::rc::Rc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use repriseapp::keys::sheet_input;
use repriseapp::oneshot::{self, ErrorBox};
use repriseapp::{App, Child, Ctx, Event, Reply, WinId};
use reprisekit::access::Tree;
use reprisekit::controls::propsheet;
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;
use reprisekit::tr;

use sheet::{Local, Outcome, Sheet, System};

#[derive(Default)]
struct DateTimeApp {
    sheet: Option<Sheet>,
    system: System,
    zones: Vec<String>,
    /// An Apply under way on the worker thread, and whether an OK started
    /// it (the sheet closes when it succeeds).
    applying: Option<(Receiver<Result<System, timedate::Failure>>, bool)>,
    /// timedated's zone each time it changes.
    zones_moved: Option<Receiver<String>>,
    waker: Option<repriseapp::Waker>,
    error: ErrorBox,
    /// The renderer and forms theme the box is sized with: `children` has no
    /// `Ctx`, so they are kept from `start` and the last event.
    dialog_text: Option<Rc<TextRenderer>>,
    forms: Theme,
    /// The client size last asked for the main window.
    asked: (i32, i32),
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
                Err(failure) => {
                    // The settings may be half changed: read what stuck.
                    if let Ok((system, _)) = timedate::read() {
                        self.system = system;
                    }
                    if let Some(s) = &mut self.sheet {
                        if failure.refused() {
                            s.refused(&self.system, now());
                        } else {
                            s.unapplied(&self.system);
                        }
                    }
                    if let Some(text) = failure.message() {
                        self.error.show(&tr!("title"), text, None);
                    }
                }
            }
        }
        if let Some(zone) = self
            .zones_moved
            .as_ref()
            .and_then(|rx| rx.try_iter().last())
            && zone != self.system.zone
        {
            self.system.zone = zone;
            if let Some(s) = &mut self.sheet {
                s.zone_moved(&self.system.zone);
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
}

impl App for DateTimeApp {
    fn title(&self) -> String {
        tr!("title")
    }

    fn app_id(&self) -> String {
        "reprise.datetime".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        sheet::CLIENT
    }

    fn children(&self) -> Vec<Child> {
        (self.dialog_text.as_ref())
            .and_then(|text| self.error.child(text, &self.forms))
            .into_iter()
            .collect()
    }

    fn start(&mut self, waker: repriseapp::Waker) -> repriseapp::Start {
        self.dialog_text = Some(repriseapp::start_text());
        self.asked = sheet::CLIENT;
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
                self.system.auto_zone = repriseconf::current().auto_time_zone;
            }
        }
        start_ticker(waker.clone());
        let (tx, rx) = channel();
        timedate::follow_zone(tx, waker.clone());
        self.zones_moved = Some(rx);
        self.waker = Some(waker);
        repriseapp::Start::Continue
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.dialog_text = Some(ctx.text.clone());
        self.forms = ctx.forms();
        let mut reply = self.dispatch(win, ev, ctx);
        let size = self.forms.text.size(sheet::CLIENT);
        reply.ask_size(&mut self.asked, size);
        reply
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        self.dialog_text = Some(ctx.text.clone());
        let forms = ctx.forms();
        if self.error.owns(win) {
            self.error.draw(c, &forms, &ctx.text);
            return;
        }
        let s = self.ensure();
        s.fit(forms.text);
        c.fill_rect(c.bounds(), forms.colors.button_face);
        propsheet::draw(
            c,
            &ctx.text,
            &Paint {
                s: 1,
                ..forms.paint()
            },
            &s.sheet,
            true,
            forms.fonts.size_px,
        );
    }
}

impl repriseapp::Extended for DateTimeApp {
    fn fixed(&self) -> bool {
        true
    }

    fn reader_form(&mut self, win: WinId) -> Option<repriseapp::ReaderForm<'_>> {
        let s = self.sheet.as_mut().filter(|_| !self.error.owns(win))?;
        Some(repriseapp::ReaderForm::Sheet(&mut s.sheet))
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let forms = ctx.forms();
        if self.error.owns(win) {
            self.error.access(tree, &ctx.text, &forms);
        } else if let Some(s) = &self.sheet {
            oneshot::access_sheet(&s.sheet, tree, &ctx.text, forms.fonts.size_px);
        }
    }
}

impl DateTimeApp {
    fn dispatch(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        if matches!(ev, Event::Wake) {
            return self.wake();
        }
        if self.error.owns(win) {
            let Some(text) = &self.dialog_text else {
                return Reply::none();
            };
            return self.error.event(win, &ev, text, &self.forms);
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
}

/// Runs the program; `args` are the command line after the program's name.
pub fn main(_args: Vec<std::ffi::OsString>) {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    repriseapp::run(Box::new(DateTimeApp::default())).unwrap_or_else(|e| e.exit());
}
