//! reprise-updates: Software Updates.
//!
//! One process for the session (autostarted with `--tray`, the window hidden); plain
//! `reprise-updates` shows the window and checks at once. Closing the window hides it; an install
//! carries on behind it.

mod flatpak;
mod fwupd;
mod model;
mod packagekit;
mod tray;
mod window;
mod worker;

use std::sync::mpsc::Receiver;

use repriseapp::keys::{dialog_key, form_input};
use repriseapp::{App, BTN_LEFT, Ctx, Event, PointerInput, Reply, WinId};
use reprisekit::access::Tree;
use reprisekit::raster::Canvas;
use reprisekit::theme::TextUnits;

use model::{Announced, Phase, Restart, Update};

const ICON: &str = "system-software-update";

struct UpdatesApp {
    tray: Option<reprisetray::TrayHandle>,
    msgs: Option<Receiver<reprisetray::Msg>>,
    worker: Option<Receiver<worker::Msg>>,
    jobs: Option<worker::Jobs>,
    phase: Phase,
    updates: Vec<Update>,
    announced: Announced,
    /// An install just ended: its result stays up through the check that follows it.
    hold: bool,
    tray_state: Option<model::TrayLook>,

    window: window::Window,
    icon_loaded: bool,
    hidden: bool,
    tray_only: bool,
    /// The text size at the last event, which `initial_size` grows the window by.
    units: TextUnits,
    /// The client size last asked for the window.
    asked: (i32, i32),
}

impl UpdatesApp {
    fn new(tray_only: bool) -> UpdatesApp {
        UpdatesApp {
            tray: None,
            msgs: None,
            worker: None,
            jobs: None,
            phase: Phase::Unchecked,
            updates: Vec::new(),
            announced: Announced::default(),
            hold: false,
            tray_state: None,
            window: window::Window::new(None),
            units: TextUnits::ONE,
            asked: window::CLIENT,
            icon_loaded: false,
            hidden: tray_only,
            tray_only,
        }
    }

    fn notify(&self, (summary, body): (String, String), clickable: bool) {
        if let Some(t) = &self.tray {
            if clickable {
                t.notify_clickable(&summary, &body, 0);
            } else {
                t.notify(&summary, &body);
            }
        }
    }

    fn push_state(&mut self) {
        let next = model::tray_look(&self.phase, &self.updates);
        if self.tray_state.as_ref() == Some(&next) {
            return;
        }
        if let Some(t) = &self.tray {
            t.set_state(tray::shown(&next));
        }
        self.tray_state = Some(next);
    }

    /// A restart or log off an install asked for holds until it happens: later checks leave it up.
    fn waits_on_restart(&self) -> bool {
        matches!(
            self.phase,
            Phase::Installed(Restart::System | Restart::Session)
        )
    }

    fn check(&mut self) {
        if self.phase.busy() {
            return;
        }
        if let Some(j) = &self.jobs {
            j.send(worker::Job::Check {
                refresh: true,
                manual: true,
            });
        }
    }

    fn install(&mut self) {
        let picked: Vec<Update> = self
            .updates
            .iter()
            .filter(|u| self.window.wanted(u))
            .cloned()
            .collect();
        if self.phase.busy() || picked.is_empty() {
            return;
        }
        self.phase = Phase::Installing {
            fraction: None,
            doing: "Preparing to install".into(),
            request: None,
        };
        if let Some(j) = &self.jobs {
            j.send(worker::Job::Install(picked));
        }
    }

    fn worker_msg(&mut self, m: worker::Msg) {
        match m {
            worker::Msg::Checking => {
                if !self.hold && !self.waits_on_restart() {
                    self.phase = Phase::Checking;
                }
            }
            worker::Msg::Checked(found) => {
                // A check already running when Install was pressed ends before the install starts:
                // the phase stays Installing, so its progress keeps showing.
                let keep = std::mem::take(&mut self.hold)
                    || self.waits_on_restart()
                    || matches!(self.phase, Phase::Installing { .. });
                let (updates, failed) = model::merge(&self.updates, found);
                // Only a background find balloons: with the window up, the list says it.
                if self.announced.news(&updates) && self.hidden {
                    self.notify(model::news_balloon(&updates), true);
                }
                self.updates = updates;
                if let Some(why) = &failed {
                    log::warn!("checking for updates: {why}");
                }
                if !keep {
                    self.phase = match failed {
                        Some(why) => Phase::Failed(model::check_failed(&why)),
                        None => Phase::Ready,
                    };
                }
            }
            worker::Msg::Progress { fraction, doing } => {
                if let Phase::Installing {
                    fraction: f,
                    doing: d,
                    ..
                } = &mut self.phase
                {
                    *f = fraction.or(*f);
                    *d = doing;
                }
            }
            worker::Msg::Request(r) => {
                // One to do now shows in the window; it balloons too when the window is hidden. One
                // that finishes the update afterwards always balloons, as the install ends with it.
                if let Some(r) = &r
                    && (self.hidden || !r.now)
                {
                    self.notify(model::request_balloon(r), true);
                }
                if let Phase::Installing { request, .. } = &mut self.phase {
                    *request = r.filter(|r| r.now).map(|r| r.message);
                }
            }
            worker::Msg::Installed(result) => {
                self.hold = true;
                match result {
                    Ok(restart) => {
                        if self.hidden || restart != Restart::None {
                            self.notify(model::installed_balloon(restart), true);
                        }
                        self.phase = Phase::Installed(restart);
                    }
                    Err(worker::Failure::Dismissed) => self.phase = Phase::Ready,
                    Err(worker::Failure::Text(why)) => {
                        log::warn!("installing updates: {why}");
                        let message = model::install_failed(&why);
                        if self.hidden {
                            self.notify(("Updates were not installed".into(), why), true);
                        }
                        self.phase = Phase::Failed(message);
                    }
                }
            }
        }
    }

    fn wake(&mut self) -> Reply {
        while let Some(m) = self.worker.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.worker_msg(m);
        }
        let msgs: Vec<tray::TrayMsg> = self
            .msgs
            .as_ref()
            .map(|rx| rx.try_iter().filter_map(tray::translate).collect())
            .unwrap_or_default();
        for msg in msgs {
            match msg {
                tray::TrayMsg::Open => self.hidden = false,
                tray::TrayMsg::Check => self.check(),
            }
        }
        self.push_state();
        Reply {
            redraw: true,
            activate: self.tray.as_ref().and_then(|t| t.take_activation_token()),
            ..Reply::none()
        }
    }

    fn load_icons(&mut self, ctx: &mut Ctx) {
        if self.icon_loaded {
            return;
        }
        self.icon_loaded = true;
        if let Some(i) = ctx
            .icons
            .get(ICON, 32)
            .or_else(|| ctx.icons.get("software-update-available", 32))
        {
            self.window.set_icon(i);
        }
    }

    fn sync_window(&mut self, ctx: &Ctx) {
        self.window
            .sync(&self.phase, &self.updates, &ctx.text, &ctx.forms());
    }

    fn list_input(&mut self, ev: &Event, ctx: &Ctx) -> Option<window::Outcome> {
        use window::ListInput;
        let cm = ctx.theme.controls();
        let input = match *ev {
            Event::Key(ref k) => return self.window.list_key(&dialog_key(k)?, &cm),
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                ..
            }) => ListInput::Press(x, y),
            Event::Pointer(PointerInput::Motion { x, y }) => ListInput::Motion(x, y),
            Event::Pointer(PointerInput::Release {
                x,
                y,
                button: BTN_LEFT,
                ..
            }) => ListInput::Release(x, y),
            Event::Pointer(PointerInput::Scroll { x, y, rows }) => ListInput::Scroll(x, y, rows),
            _ => return None,
        };
        self.window.list_pointer(input, &cm)
    }

    fn restart(&self) -> Reply {
        let config = std::fs::read_to_string(repriseconf::config_path()).unwrap_or_default();
        let cmd = model::reboot_command(&config);
        Reply {
            spawn: vec![vec!["sh".into(), "-c".into(), cmd.into()]],
            ..Reply::none()
        }
    }
}

impl App for UpdatesApp {
    fn title(&self) -> String {
        tray::TITLE.into()
    }

    fn app_id(&self) -> String {
        "reprise.updates".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        self.units.size(window::CLIENT)
    }

    fn text_sized(&self) -> bool {
        true
    }

    fn fixed(&self) -> bool {
        true
    }

    fn hidden(&self) -> bool {
        self.hidden
    }

    fn start(&mut self, waker: repriseapp::Waker) {
        // The tray goes first: it takes the `reprise.updates` bus name, and a second instance
        // exits here once it has raised the first.
        let first = model::tray_look(&self.phase, &self.updates);
        match tray::start(waker.clone(), &first) {
            reprisetray::Started::Running(t, msgs) => {
                self.tray = Some(t);
                self.msgs = Some(msgs);
                self.tray_state = Some(first);
            }
            reprisetray::Started::AlreadyRunning => std::process::exit(0),
            // Under `--tray` the window starts hidden, so without the item
            // nothing would be on screen to bring it back.
            reprisetray::Started::Unavailable if self.tray_only => {
                log::error!(
                    "--tray was given but the tray item could not start (no session bus?); exiting"
                );
                std::process::exit(1);
            }
            reprisetray::Started::Unavailable => {}
        }
        let (rx, jobs) = worker::start(waker, !self.tray_only);
        self.worker = Some(rx);
        self.jobs = Some(jobs);
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.units = ctx.forms().text;
        let mut reply = self.dispatch(win, ev, ctx);
        let size = self.units.size(window::CLIENT);
        if size != self.asked {
            self.asked = size;
            reply.size = Some(size);
            reply.redraw = true;
        }
        reply
    }

    fn access(&mut self, _win: WinId, tree: &mut Tree, ctx: &Ctx) {
        self.window.access(
            tree,
            &ctx.text,
            ctx.forms().fonts.size_px,
            &ctx.theme.controls(),
        );
    }

    fn draw(&mut self, _win: WinId, c: &mut Canvas, ctx: &Ctx) {
        self.sync_window(ctx);
        self.window
            .draw(c, &ctx.text, &ctx.theme, &ctx.forms(), &self.phase);
    }
}

impl UpdatesApp {
    fn dispatch(&mut self, _win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.load_icons(ctx);
        if matches!(ev, Event::Wake) {
            let reply = self.wake();
            self.sync_window(ctx);
            return reply;
        }
        self.sync_window(ctx);
        let out = match self.list_input(&ev, ctx) {
            Some(out) => out,
            None => match ev {
                Event::CloseRequest => window::Outcome::Close,
                ev => match form_input(&mut self.window.form, &ev, ctx) {
                    Some(ev) => self.window.event(ev),
                    None => return Reply::none(),
                },
            },
        };
        let reply = match out {
            window::Outcome::None => return Reply::none(),
            window::Outcome::Redraw => Reply::redraw(),
            window::Outcome::Check => {
                self.check();
                Reply::redraw()
            }
            window::Outcome::Install => {
                self.install();
                self.push_state();
                Reply::redraw()
            }
            window::Outcome::Restart => self.restart(),
            window::Outcome::Close => {
                self.hidden = true;
                Reply::redraw()
            }
        };
        self.sync_window(ctx);
        reply
    }
}

fn main() {
    reprisebus::journal::init_logger("error");
    let tray_only = std::env::args().any(|a| a == "--tray");
    repriseapp::run(Box::new(UpdatesApp::new(tray_only)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{Group, Kind, Origin};

    fn update(name: &str) -> Update {
        Update {
            id: format!("{name};2;x86_64;core"),
            name: name.into(),
            current: String::new(),
            version: "2".into(),
            summary: String::new(),
            kind: Kind::Normal,
            origin: Origin::Package,
        }
    }

    fn found(r: Result<Vec<Update>, &str>) -> worker::Msg {
        worker::Msg::Checked(vec![(Group::Packages, r.map_err(String::from))])
    }

    #[test]
    fn an_install_result_outlasts_the_check_after_it() {
        let mut app = UpdatesApp::new(true);
        app.worker_msg(found(Ok(vec![update("a")])));
        assert_eq!(app.phase, Phase::Ready);
        app.install();
        assert!(app.phase.busy());
        app.worker_msg(worker::Msg::Progress {
            fraction: Some(0.25),
            doing: "Installing a".into(),
        });
        app.worker_msg(worker::Msg::Progress {
            fraction: None,
            doing: "Installing b".into(),
        });
        assert_eq!(
            app.phase,
            Phase::Installing {
                fraction: Some(0.25),
                doing: "Installing b".into(),
                request: None,
            }
        );
        app.worker_msg(worker::Msg::Installed(Ok(Restart::None)));
        app.worker_msg(worker::Msg::Checking);
        app.worker_msg(found(Ok(Vec::new())));
        assert_eq!(app.phase, Phase::Installed(Restart::None));
        app.worker_msg(worker::Msg::Checking);
        assert_eq!(app.phase, Phase::Checking);
        app.worker_msg(found(Ok(Vec::new())));
        assert_eq!(app.phase, Phase::Ready);
    }

    #[test]
    fn a_pending_restart_survives_later_checks() {
        let mut app = UpdatesApp::new(true);
        app.worker_msg(worker::Msg::Installed(Ok(Restart::System)));
        for _ in 0..2 {
            app.worker_msg(worker::Msg::Checking);
            app.worker_msg(found(Ok(Vec::new())));
        }
        assert_eq!(app.phase, Phase::Installed(Restart::System));
        assert!(model::tray_look(&app.phase, &app.updates).active);
    }

    #[test]
    fn a_check_ending_after_install_was_pressed_leaves_it_installing() {
        let mut app = UpdatesApp::new(true);
        app.worker_msg(found(Ok(vec![update("a")])));
        app.install();
        app.worker_msg(worker::Msg::Installed(Ok(Restart::Session)));
        app.worker_msg(found(Ok(vec![update("b")])));
        app.worker_msg(worker::Msg::Checking);
        app.install();
        app.worker_msg(found(Ok(vec![update("b")])));
        assert!(app.phase.busy(), "still installing, not {:?}", app.phase);
        app.worker_msg(worker::Msg::Progress {
            fraction: Some(0.5),
            doing: "Installing b".into(),
        });
        assert_eq!(
            app.phase,
            Phase::Installing {
                fraction: Some(0.5),
                doing: "Installing b".into(),
                request: None,
            }
        );
    }

    #[test]
    fn a_dismissed_install_returns_to_the_list_and_a_refused_one_says_so() {
        let mut app = UpdatesApp::new(true);
        app.worker_msg(found(Ok(vec![update("a")])));
        app.install();
        app.worker_msg(worker::Msg::Installed(Err(worker::Failure::Dismissed)));
        assert_eq!(app.phase, Phase::Ready);
        assert_eq!(app.updates.len(), 1);

        app.install();
        let refused = worker::Failure::Text("Failed to obtain authentication.".into());
        app.worker_msg(worker::Msg::Installed(Err(refused)));
        assert_eq!(
            app.phase,
            Phase::Failed(
                "The updates could not be installed. Failed to obtain authentication.".into()
            )
        );
    }

    #[test]
    fn a_device_request_heads_the_window_until_the_device_carries_on() {
        let mut app = UpdatesApp::new(false);
        app.worker_msg(found(Ok(vec![update("a")])));
        app.install();
        let replug = model::Request {
            message: "Unplug Webcam and plug it back in to continue the update.".into(),
            now: true,
        };
        app.worker_msg(worker::Msg::Request(Some(replug.clone())));
        app.worker_msg(worker::Msg::Progress {
            fraction: Some(0.5),
            doing: "Waiting for you".into(),
        });
        assert_eq!(model::headline(&app.phase, &app.updates, 1), replug.message);
        app.worker_msg(worker::Msg::Request(None));
        assert!(
            model::headline(&app.phase, &app.updates, 1).starts_with("Installing updates."),
            "{:?}",
            app.phase
        );
        app.worker_msg(worker::Msg::Request(Some(model::Request {
            now: false,
            ..replug
        })));
        assert!(
            model::headline(&app.phase, &app.updates, 1).starts_with("Installing updates."),
            "a request for afterwards goes to a balloon"
        );
    }

    #[test]
    fn a_failed_check_says_so_and_keeps_the_list() {
        let mut app = UpdatesApp::new(true);
        app.worker_msg(found(Ok(vec![update("a")])));
        app.worker_msg(found(Err("No network.")));
        assert_eq!(
            app.phase,
            Phase::Failed("Reprise could not check for updates. No network.".into())
        );
        assert_eq!(app.updates.len(), 1);
    }
}
