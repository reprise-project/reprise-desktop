//! reprise-updates: Software Updates.
//!
//! One process for the session (autostarted with `--tray`, the window hidden); plain
//! `reprise-updates` shows the window and checks at once. Closing the window hides it; an install
//! carries on behind it. `--install` and `--uninstall` run the install sheet (`installer`).

#[cfg(test)]
reprisebus::test_session!();

mod flatpak;
mod fwupd;
mod installer;
mod model;
mod packagekit;
mod pkgfile;
mod restart;
mod tray;
mod window;
mod worker;

use std::path::PathBuf;
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
    /// What the restart watch last said, and `[updates] restart_notice`, reread on each config change.
    restart_due: bool,
    notice_on: bool,
    config: Option<repriseconf::ConfigWatch>,
    /// The once-a-boot mark of the restart balloon, and this boot's id.
    mark: PathBuf,
    boot: String,
    /// Asks the compositor to restart; a test puts a stand-in here.
    restarter: fn() -> zbus::Result<()>,

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
            restart_due: false,
            notice_on: repriseconf::restart_notice(),
            config: None,
            mark: restart::mark_path(),
            boot: restart::boot_id(),
            restarter: reprisebus::tray::restart,
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
            Phase::Installed(Restart::System | Restart::Session) | Phase::RestartDue
        )
    }

    /// A restart the system needs shows in the window and tray until it happens, and in a balloon
    /// once a boot. An install's result says it instead, and one running holds it until it ends.
    fn settle_restart(&mut self) {
        if !(self.restart_due && self.notice_on) {
            if self.phase == Phase::RestartDue {
                self.phase = Phase::Ready;
            }
            return;
        }
        match self.phase {
            Phase::Checking | Phase::Installing { .. } | Phase::RestartDue => return,
            Phase::Installed(Restart::System) => {
                restart::first_this_boot(&self.mark, &self.boot);
                return;
            }
            Phase::Installed(_) => self.phase = Phase::Installed(Restart::System),
            Phase::Unchecked | Phase::Ready | Phase::Failed(_) => self.phase = Phase::RestartDue,
        }
        if restart::first_this_boot(&self.mark, &self.boot)
            && let Some(t) = &self.tray
        {
            let (summary, body) = model::restart_balloon();
            t.notify_buttons(
                &summary,
                &body,
                tray::RESTART_BALLOON,
                &tray::RESTART_BUTTONS,
            );
        }
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
                // One to do now shows in the window; it balloons when the window is hidden or closes. One
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
                        let restart = if self.restart_due && self.notice_on {
                            restart.max(Restart::System)
                        } else {
                            restart
                        };
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
            worker::Msg::RestartDue(due) => self.restart_due = due,
        }
        self.settle_restart();
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
        if self.config.as_mut().is_some_and(|w| w.take_changed()) {
            self.notice_on = repriseconf::restart_notice();
            self.settle_restart();
        }
        let raised = !msgs.is_empty();
        let mut shown = None;
        for msg in msgs {
            match msg {
                tray::TrayMsg::Open => {
                    self.hidden = false;
                    shown = Some(repriseapp::MAIN);
                }
                tray::TrayMsg::Check => self.check(),
                tray::TrayMsg::Restart => self.restart(),
            }
        }
        self.push_state();
        Reply {
            redraw: true,
            activate: reprisetray::handoff_for(self.tray.as_ref(), raised, shown),
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

    /// Restart Now: the compositor's Restart, so programs get to ask about unsaved work.
    fn restart(&self) {
        if let Err(e) = (self.restarter)() {
            log::warn!("the compositor did not take the restart: {e}");
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

    fn start(&mut self, waker: repriseapp::Waker) -> repriseapp::Start {
        // The tray goes first: it takes the `reprise.updates` bus name, and a second instance
        // exits here once it has raised the first.
        let first = model::tray_look(&self.phase, &self.updates);
        match tray::start(waker.clone(), &first).or_exit(self.tray_only) {
            Ok(Some((t, msgs))) => {
                self.tray = Some(t);
                self.msgs = Some(msgs);
                self.tray_state = Some(first);
            }
            Ok(None) => {}
            Err(code) => return repriseapp::Start::Exit(code),
        }
        let w = waker.clone();
        self.config = repriseconf::watch(move || w.wake());
        let (rx, jobs) = worker::start(waker, !self.tray_only);
        self.worker = Some(rx);
        self.jobs = Some(jobs);
        repriseapp::Start::Continue
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

    fn draw(&mut self, _win: WinId, c: &mut Canvas, ctx: &Ctx) {
        self.sync_window(ctx);
        self.window
            .draw(c, &ctx.text, &ctx.theme, &ctx.forms(), &self.phase);
    }
}

impl repriseapp::Extended for UpdatesApp {
    fn text_sized(&self) -> bool {
        true
    }

    fn fixed(&self) -> bool {
        true
    }

    fn hidden(&self) -> bool {
        self.hidden
    }

    fn access(&mut self, _win: WinId, tree: &mut Tree, ctx: &Ctx) {
        self.window.access(
            tree,
            &ctx.text,
            ctx.forms().fonts.size_px,
            &ctx.theme.controls(),
        );
    }

    fn reader(&mut self, _win: WinId, act: repriseapp::Reader, ctx: &Ctx) -> bool {
        let px = ctx.forms().fonts.size_px;
        (self.window).reader(act, &ctx.text, px, &ctx.theme.controls())
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
            window::Outcome::Restart => {
                self.restart();
                Reply::none()
            }
            window::Outcome::Close => {
                if let Some(balloon) = model::closed_balloon(&self.phase) {
                    self.notify(balloon, true);
                }
                self.hidden = true;
                Reply::redraw()
            }
        };
        self.sync_window(ctx);
        reply
    }
}

/// Runs the program; `args` are the command line after the program's name. `--install FILE` and
/// `--uninstall FILE` open the install sheet instead (`installer`); without a file (a session
/// restoring the sheet) they do nothing.
pub fn main(args: Vec<std::ffi::OsString>) {
    if installer::asks_for_sheet(&args) {
        if let Some(target) = installer::target(&args) {
            installer::main(target);
        }
        return;
    }
    repriseapp::applet_main(&args, UpdatesApp::new);
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

    fn restart_app(name: &str, notice_on: bool) -> (UpdatesApp, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("reprise-updates-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let app = UpdatesApp {
            notice_on,
            mark: dir.join("restart-notice"),
            boot: "boot-1".into(),
            ..UpdatesApp::new(true)
        };
        (app, dir)
    }

    fn marked(app: &UpdatesApp) -> bool {
        std::fs::read_to_string(&app.mark).is_ok_and(|s| s.trim() == "boot-1")
    }

    #[test]
    fn a_restart_the_system_needs_holds_through_checks_until_it_goes() {
        let (mut app, dir) = restart_app("due", true);
        app.worker_msg(worker::Msg::RestartDue(true));
        assert_eq!(app.phase, Phase::RestartDue);
        assert!(marked(&app), "the balloon went up this boot");
        let look = model::tray_look(&app.phase, &app.updates);
        assert_eq!((look.icon, look.active), ("system-reboot", true));
        app.worker_msg(worker::Msg::Checking);
        app.worker_msg(found(Ok(vec![update("a")])));
        assert_eq!(app.phase, Phase::RestartDue);
        assert_eq!(app.updates.len(), 1);
        app.worker_msg(worker::Msg::RestartDue(false));
        assert_eq!(app.phase, Phase::Ready);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_restart_due_during_an_install_becomes_the_installs_restart() {
        let (mut app, dir) = restart_app("install", true);
        app.worker_msg(found(Ok(vec![update("linux")])));
        app.install();
        app.worker_msg(worker::Msg::RestartDue(true));
        assert!(app.phase.busy(), "the install's progress stays up");
        assert!(!marked(&app));
        app.worker_msg(worker::Msg::Installed(Ok(Restart::None)));
        assert_eq!(app.phase, Phase::Installed(Restart::System));
        assert!(marked(&app), "the install's balloon said it");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_restart_notice_switched_off_shows_nothing() {
        let (mut app, dir) = restart_app("off", false);
        app.worker_msg(worker::Msg::RestartDue(true));
        assert_eq!(app.phase, Phase::Unchecked);
        app.worker_msg(worker::Msg::Installed(Ok(Restart::None)));
        assert_eq!(app.phase, Phase::Installed(Restart::None));
        assert!(!marked(&app));
        app.notice_on = true;
        app.settle_restart();
        assert_eq!(app.phase, Phase::Installed(Restart::System));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    static RESTARTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn counted_restart() -> zbus::Result<()> {
        RESTARTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    #[test]
    fn restart_now_on_the_balloon_asks_the_compositor_and_later_does_not() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = UpdatesApp {
            msgs: Some(rx),
            restarter: counted_restart,
            ..UpdatesApp::new(true)
        };
        tx.send(reprisetray::Msg::BalloonButton(tray::RESTART_BALLOON, 1))
            .unwrap();
        app.wake();
        assert_eq!(RESTARTS.load(std::sync::atomic::Ordering::SeqCst), 0);
        tx.send(reprisetray::Msg::BalloonButton(tray::RESTART_BALLOON, 0))
            .unwrap();
        app.wake();
        assert_eq!(RESTARTS.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    /// A wake that shows no window leaves a second instance's handoff for the wake whose raise does.
    #[test]
    fn the_handoff_waits_for_the_wake_that_shows_the_window() {
        let handoff = reprisebus::instance::Handoff {
            token: Some("t".into()),
            by_key: true,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = UpdatesApp {
            tray: Some(reprisetray::TrayHandle::holding(handoff.clone())),
            msgs: Some(rx),
            ..UpdatesApp::new(true)
        };
        assert_eq!(app.wake().activate, None);
        tx.send(reprisetray::Rows::clicked(&tray::TrayMsg::Open))
            .unwrap();
        assert_eq!(app.wake().activate, Some((repriseapp::MAIN, handoff)));
    }
}
