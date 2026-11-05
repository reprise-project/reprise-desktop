//! reprise-phone: the phone icon for the devices KDE Connect reaches. No window; the process is
//! autostarted for the session where kdeconnectd is installed, and its icon shows only while
//! kdeconnectd runs.

#[cfg(test)]
reprisebus::test_session!();

mod pick;
mod tray;

use std::sync::mpsc::Receiver;

use repriseapp::{App, Ctx, Event, Reply, WinId};
use reprisebus::kdeconnect::{self, Device};
use reprisekit::raster::Canvas;

use tray::TrayMsg;

/// KDE Connect's own window, where pairing and the plugins are set up.
const SETTINGS_ARGV: &str = "kdeconnect-app";
const SETTINGS_APP: &str = "org.kde.kdeconnect.app";

#[derive(Default)]
struct PhoneApp {
    tray: Option<reprisetray::TrayHandle>,
    msgs: Option<Receiver<reprisetray::Msg>>,
    lists: Option<Receiver<Option<Vec<Device>>>>,
    /// None while kdeconnectd is not on the bus.
    devices: Option<Vec<Device>>,
    tray_state: Option<reprisetray::Shown>,
}

impl PhoneApp {
    fn devices(&self) -> &[Device] {
        self.devices.as_deref().unwrap_or_default()
    }

    fn push_state(&mut self) {
        let next = tray::shown(self.devices.as_deref());
        if self.tray_state.as_ref() == Some(&next) {
            return;
        }
        if let Some(t) = &self.tray {
            t.set_state(next.clone());
        }
        self.tray_state = Some(next);
    }

    fn act(&mut self, m: TrayMsg) {
        match m {
            TrayMsg::Settings => self.open_settings(),
            TrayMsg::Ring(id) => on_bus("ring", move |conn| {
                reprisebus::wait(kdeconnect::ring(conn, &id))
            }),
            TrayMsg::Send(id) => {
                let name = (self.devices().iter())
                    .find(|d| d.id == id)
                    .map_or_else(|| id.clone(), |d| d.name.clone());
                on_bus("send", move |conn| {
                    let title = format!("Send Files to {name}");
                    let urls = reprisebus::wait(pick::files(conn, &title, "Send"))?;
                    if urls.is_empty() {
                        return Ok(());
                    }
                    reprisebus::wait(kdeconnect::share(conn, &id, &urls))
                });
            }
            TrayMsg::Status(_) | TrayMsg::NoDevices => {}
        }
    }

    /// KDE Connect's window, presented with the token the click left. Joining its app scope waits
    /// on the session bus, so the start runs on a thread.
    fn open_settings(&self) {
        let token = self.tray.as_ref().and_then(|t| t.take_activation_token());
        let spawned = std::thread::Builder::new()
            .name("reprise-phone-settings".into())
            .spawn(move || {
                let mut cmd = std::process::Command::new(SETTINGS_ARGV);
                if let Some(t) = token {
                    cmd.env("XDG_ACTIVATION_TOKEN", t);
                }
                if let Err(e) = reprisefs::scope::hold(&mut cmd, SETTINGS_APP) {
                    log::warn!("{SETTINGS_ARGV} runs unscoped: {e}");
                }
                match cmd.spawn() {
                    Ok(mut child) => drop(child.wait()),
                    Err(e) => log::warn!("could not start {SETTINGS_ARGV}: {e}"),
                }
            });
        if let Err(e) = spawned {
            log::warn!("could not start {SETTINGS_ARGV}: {e}");
        }
    }

    fn wake(&mut self) -> Reply {
        let mut changed = false;
        while let Some(list) = self.lists.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.devices = list;
            changed = true;
        }
        if changed {
            self.push_state();
        }
        while let Some(raw) = self.msgs.as_ref().and_then(|rx| rx.try_recv().ok()) {
            if let Some(m) = tray::translate(raw, self.devices()) {
                self.act(m);
            }
        }
        Reply::none()
    }
}

/// Runs `call` on the session bus on a thread of its own, logging its failure.
fn on_bus(
    what: &'static str,
    call: impl FnOnce(&zbus::Connection) -> zbus::Result<()> + Send + 'static,
) {
    let spawned = std::thread::Builder::new()
        .name(format!("reprise-phone-{what}"))
        .spawn(move || {
            if let Err(e) = reprisebus::session().and_then(|c| call(&c)) {
                log::warn!("phone {what}: {e}");
            }
        });
    if let Err(e) = spawned {
        log::warn!("could not start {what}: {e}");
    }
}

impl App for PhoneApp {
    fn title(&self) -> String {
        tray::TITLE.into()
    }

    fn app_id(&self) -> String {
        tray::BUS_NAME.into()
    }

    fn initial_size(&self) -> (i32, i32) {
        (1, 1)
    }

    fn start(&mut self, waker: repriseapp::Waker) -> repriseapp::Start {
        // The tray first: it owns the bus name, so a second instance exits here.
        match tray::start(waker.clone()).or_exit(true) {
            Ok(Some((t, msgs))) => {
                self.tray = Some(t);
                self.msgs = Some(msgs);
            }
            Ok(None) => {}
            Err(code) => return repriseapp::Start::Exit(code),
        }
        let (poster, rx) = repriseapp::Poster::channel(waker);
        kdeconnect::follow(move |list| poster.post(list));
        self.lists = Some(rx);
        repriseapp::Start::Continue
    }

    fn event(&mut self, _win: WinId, ev: Event, _ctx: &mut Ctx) -> Reply {
        match ev {
            Event::Wake => self.wake(),
            _ => Reply::none(),
        }
    }

    fn draw(&mut self, _win: WinId, _c: &mut Canvas, _ctx: &Ctx) {}
}

impl repriseapp::Extended for PhoneApp {
    fn hidden(&self) -> bool {
        true
    }
}

/// Runs the program; `args` are the command line after the program's name.
pub fn main(args: Vec<std::ffi::OsString>) {
    repriseapp::applet_main(&args, |_| PhoneApp::default());
}
