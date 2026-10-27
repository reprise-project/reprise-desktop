//! Power Options: the Reprise battery icon and power settings.
//!
//! One process: a tray icon that follows UPower's display device, the low and critical battery alarms,
//! and the Power Options Properties sheet behind it, which edits `config.toml`'s `[idle]`,
//! `[idle.battery]` and `[power]` keys. The sheet and the tray menu also switch power-profiles-daemon's
//! profile, when the daemon is there. `--tray` starts with the sheet hidden (the autostart entry); plain
//! `reprise-power` shows it. Closing the sheet hides it; only Exit in the tray menu quits. The compositor
//! does the acting: the timeouts, lid and buttons are its, read from the same file.

mod alarms;
mod logind;
mod profiles;
mod settings;
mod sheet;
mod tray;
mod upower;

use repriseapp::keys::sheet_input;
use repriseapp::oneshot::{self, MsgBox};
use repriseapp::{App, Child, Ctx, Event, Reply, WinId};
use reprisekit::access::Tree;
use reprisekit::art::powericon;
use reprisekit::controls::propsheet;
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;
use std::path::PathBuf;
use std::rc::Rc;

use settings::Settings;
use upower::{Power, State};

#[derive(Default)]
struct PowerApp {
    tray: Option<reprisetray::TrayHandle>,
    msgs: Option<std::sync::mpsc::Receiver<reprisetray::Msg>>,
    snapshots: Option<std::sync::mpsc::Receiver<Power>>,
    power: Power,
    profile_snapshots: Option<std::sync::mpsc::Receiver<profiles::Profiles>>,
    profiles: profiles::Profiles,
    setter: Option<profiles::Setter>,
    settings: Settings,
    commands: settings::Commands,
    path: PathBuf,
    alarms: alarms::Alarms,
    tray_state: Option<reprisetray::Shown>,
    sheet: Option<sheet::Sheet>,
    /// The sheet was built before an event gave us a `&mut Ctx` to read the icon from, so it draws a placeholder and wants rebuilding.
    iconless: bool,
    hidden: bool,
    tray_only: bool,
    error: Option<MsgBox>,
    /// The renderer and forms theme the box and window are sized with: `children` and
    /// `initial_size` have no `Ctx`, so they are kept from `start` and the last event.
    dialog_text: Option<Rc<TextRenderer>>,
    forms: Theme,
    /// The client size last asked for the main window.
    asked: (i32, i32),
}

const ERROR_WIN: WinId = WinId::named("error");

/// The icon for a snapshot under these alarm thresholds, for the tray and the Power Meter alike; an alarm at 0 is off, never "low".
pub fn icon_for(p: &Power, s: &Settings) -> powericon::Source {
    match &p.battery {
        None => powericon::Source::Ac,
        Some(b) => powericon::Source::Battery {
            percent: b.percent,
            charging: b.state == State::Charging,
            low: i64::from(b.percent) <= s.battery_low && s.battery_low > 0,
            critical: i64::from(b.percent) <= s.battery_critical && s.battery_critical > 0,
        },
    }
}

impl PowerApp {
    fn push_state(&mut self) {
        let next = tray::shown(
            powericon::draw(icon_for(&self.power, &self.settings)).to_argb_be(),
            tray::tooltip(&self.power),
            self.power.battery.is_some() || self.settings.always_show_icon,
            &self.profiles,
        );
        if self.tray_state.as_ref() == Some(&next) {
            return;
        }
        if let Some(tray) = &self.tray {
            tray.set_state(next.clone());
        }
        self.tray_state = Some(next);
    }

    /// Feed the alarms; a balloon goes out at once, and the critical action comes back as the command
    /// for the caller's `Reply::spawn`. A critical sleep logind cannot do shuts down instead. A shutdown
    /// goes to the compositor, which closes the programs first; the command runs here only when it does not answer.
    fn check_alarms(&mut self) -> Option<String> {
        let low = self.settings.battery_low.max(0) as u32;
        let critical = self.settings.battery_critical.max(0) as u32;
        match self.alarms.feed(&self.power, low, critical) {
            None => None,
            Some(alarms::Alarm::Low(pct)) => {
                log::info!("low battery: {pct}%");
                if let Some(t) = &self.tray {
                    t.notify(
                        "Low battery",
                        &format!("{pct}% of battery power remains. Plug the computer in."),
                    );
                }
                None
            }
            Some(alarms::Alarm::Critical(pct)) => {
                let chosen = &self.settings.critical_action;
                let action = settings::critical_action(chosen, logind::can);
                if action != chosen {
                    log::warn!("logind cannot {chosen}; shutting down instead");
                }
                log::info!("critical battery: {pct}%");
                if let Some(t) = &self.tray {
                    t.notify(
                        "Critical battery",
                        &format!(
                            "{pct}% of battery power remains. The computer is about to {}.",
                            critical_verb(action)
                        ),
                    );
                }
                if action == "turnoff" {
                    match reprisebus::tray::critical_turn_off() {
                        Ok(()) => return None,
                        Err(e) => log::warn!("compositor did not take the shutdown: {e}"),
                    }
                }
                self.commands.for_action(action)
            }
        }
    }

    fn build(&mut self, icon: Option<Option<Canvas>>) {
        if self.sheet.is_some() && !(self.iconless && icon.is_some()) {
            return;
        }
        self.iconless = icon.is_none();
        let mut s = sheet::Sheet::new(&self.settings, &self.power, &self.profiles, icon.flatten());
        if let Some(old) = &self.sheet {
            s.open_tab(old.sheet.tabs.open);
        }
        self.sheet = Some(s);
    }

    fn ensure(&mut self, ctx: &mut Ctx) {
        let icon = ctx
            .icons
            .get("preferences-system-power-management", 32)
            .or_else(|| ctx.icons.get("battery", 32));
        self.build(Some(icon));
    }

    fn reload_settings(&mut self) {
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        if settings::load(&text) != self.settings || settings::commands(&text) != self.commands {
            self.take_settings(&text);
        }
    }

    fn take_settings(&mut self, text: &str) {
        self.settings = settings::load(text);
        self.commands = settings::commands(text);
        if let Some(s) = &mut self.sheet {
            s.refresh_settings(&self.settings);
        }
    }

    /// Write the keys `want` moved off `self.settings` (the sheet's baseline) into the file, keeping
    /// everything else. On success the written file is the next baseline, other writers' keys included.
    /// On a parse or write failure the file is untouched and the sheet gets its baseline back, so it shows the values as unapplied.
    fn save(&mut self, want: &Settings) -> bool {
        let base = &self.settings;
        let written = match repriseconf::edit(&self.path, |text| settings::save(text, want, base)) {
            Ok(Ok(text)) => Some(text),
            Err(e) => {
                log::error!("cannot write {}: {e}", self.path.display());
                None
            }
            Ok(Err(e)) => {
                log::error!("{} does not parse ({e}); not writing", self.path.display());
                None
            }
        };
        let ok = written.is_some();
        if let Some(text) = written {
            self.take_settings(&text);
            self.push_state();
        } else {
            if let Some(s) = &mut self.sheet {
                s.unapplied(&self.settings);
            }
            self.error = Some(MsgBox::new(
                "Power Options",
                reprisekit::controls::msgbox::Fields {
                    text: format!("{} could not be written.", self.path.display()),
                    icon: None,
                    buttons: vec!["OK".into()],
                    default: 0,
                    min_text_h: None,
                    min_w: 0,
                    max_w: repriseapp::dialog_max_w(),
                },
            ));
        }
        ok
    }

    /// Ask the daemon for `profile` and show it as on until the daemon says otherwise.
    fn set_profile(&mut self, profile: String) {
        if let Some(setter) = &self.setter {
            setter.set(profile.clone());
        }
        self.profiles.active = profile;
        if let Some(s) = &mut self.sheet {
            s.refresh_profiles(&self.profiles);
        }
        self.push_state();
    }

    /// Write `want` when it moved, then switch to `profile` if one was picked. False when the write failed.
    fn apply(&mut self, want: &Settings, profile: Option<String>) -> bool {
        let saved = *want == self.settings || self.save(want);
        if let Some(p) = profile {
            self.set_profile(p);
        }
        saved
    }

    fn error_event(&mut self, ev: Event) -> Reply {
        let (Some(b), Some(text)) = (&mut self.error, &self.dialog_text) else {
            return Reply::none();
        };
        match b.event(&ev, text, &self.forms) {
            oneshot::Outcome::Closed(_) => {
                self.error = None;
                Reply::redraw_all()
            }
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::None => Reply::none(),
        }
    }

    fn outcome(&mut self, out: sheet::Outcome) -> Reply {
        match out {
            sheet::Outcome::None => Reply::none(),
            sheet::Outcome::Redraw => Reply::redraw(),
            sheet::Outcome::Apply(want, profile) => {
                self.apply(&want, profile);
                Reply::redraw()
            }
            sheet::Outcome::ApplyAndClose(want, profile) => {
                if self.apply(&want, profile) {
                    self.hidden = true;
                }
                Reply::redraw()
            }
            sheet::Outcome::Close => {
                self.hidden = true;
                Reply::redraw()
            }
        }
    }

    /// Drain the UPower, profile and tray channels. Quits only on the tray's Exit.
    fn wake(&mut self) -> Reply {
        let fresh_profiles = self
            .profile_snapshots
            .as_ref()
            .and_then(|rx| rx.try_iter().last());
        if let Some(p) = fresh_profiles {
            self.profiles = p;
            if let Some(s) = &mut self.sheet {
                s.refresh_profiles(&self.profiles);
            }
            self.push_state();
        }
        let mut fresh = None;
        if let Some(rx) = &self.snapshots {
            while let Ok(p) = rx.try_recv() {
                fresh = Some(p);
            }
        }
        let mut spawn = None;
        if let Some(p) = fresh {
            self.power = p;
            if let Some(s) = &mut self.sheet {
                s.refresh_power(&self.power);
            }
            // The alarms and the icon go by the file as it is now, other writers' edits included.
            self.reload_settings();
            self.push_state();
            spawn = self.check_alarms();
        }
        let msgs: Vec<tray::TrayMsg> = self
            .msgs
            .as_ref()
            .map(|rx| {
                rx.try_iter()
                    .filter_map(|m| tray::translate(m, &self.profiles))
                    .collect()
            })
            .unwrap_or_default();
        let raised = !msgs.is_empty();
        let mut shown = None;
        for msg in msgs {
            match msg {
                tray::TrayMsg::Exit => {
                    return Reply {
                        quit: true,
                        ..Reply::none()
                    };
                }
                tray::TrayMsg::Profile(i) => {
                    if let Some(p) = self.profiles.available.get(i).cloned() {
                        self.set_profile(p);
                    }
                }
                tray::TrayMsg::OpenMeter | tray::TrayMsg::OpenProperties => {
                    self.reload_settings();
                    self.build(None);
                    if let Some(s) = &mut self.sheet {
                        s.open_tab(if msg == tray::TrayMsg::OpenMeter {
                            sheet::TAB_METER
                        } else {
                            sheet::TAB_SCHEMES
                        });
                    }
                    self.hidden = false;
                    shown = Some(repriseapp::MAIN);
                }
            }
        }
        let spawn: Vec<Vec<std::ffi::OsString>> = spawn
            .map(|cmd| vec!["sh".into(), "-c".into(), cmd.into()])
            .into_iter()
            .collect();
        Reply {
            redraw: true,
            spawn,
            activate: reprisetray::handoff_for(self.tray.as_ref(), raised, shown),
            ..Reply::none()
        }
    }
}

fn critical_verb(action: &str) -> &'static str {
    match action {
        "standby" => "stand by",
        "hibernate" => "hibernate",
        "turnoff" => "shut down",
        _ => "keep running",
    }
}

impl App for PowerApp {
    fn title(&self) -> String {
        "Power Options Properties".into()
    }

    fn app_id(&self) -> String {
        "reprise.power".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        self.forms.text.size(sheet::CLIENT)
    }

    fn children(&self) -> Vec<Child> {
        match (&self.error, &self.dialog_text) {
            (Some(b), Some(text)) => vec![b.child(ERROR_WIN, text, &self.forms)],
            _ => Vec::new(),
        }
    }

    fn start(&mut self, waker: repriseapp::Waker) -> repriseapp::Start {
        self.dialog_text = Some(repriseapp::start_text());
        self.asked = sheet::CLIENT;
        // The tray goes first: it takes the `reprise.power` bus name, and a second instance exits here once it has raised the first.
        match tray::start(waker.clone()).or_exit(self.tray_only) {
            Ok(Some((tray, msgs))) => {
                self.tray = Some(tray);
                self.msgs = Some(msgs);
            }
            Ok(None) => {}
            Err(code) => return repriseapp::Start::Exit(code),
        }
        self.reload_settings();
        let (profile_snapshots, setter) = profiles::watch(waker.clone());
        self.profile_snapshots = Some(profile_snapshots);
        self.setter = Some(setter);
        self.snapshots = Some(upower::watch(waker));
        repriseapp::Start::Continue
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.dialog_text = Some(ctx.text.clone());
        self.forms = ctx.forms();
        let mut reply = self.dispatch(win, ev, ctx);
        let size = self.forms.text.size(sheet::CLIENT);
        if size != self.asked {
            self.asked = size;
            reply.size = Some(size);
            reply.redraw = true;
        }
        reply
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        self.dialog_text = Some(ctx.text.clone());
        let forms = ctx.forms();
        if win == ERROR_WIN {
            if let (Some(b), Some(text)) = (&self.error, &self.dialog_text) {
                b.draw(c, &forms, text);
            }
            return;
        }
        self.build(None);
        c.fill_rect(c.bounds(), forms.colors.button_face);
        if let Some(s) = &self.sheet {
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
}

impl repriseapp::Extended for PowerApp {
    fn fixed(&self) -> bool {
        true
    }

    fn hidden(&self) -> bool {
        self.hidden
    }

    fn reader_form(&mut self, win: WinId) -> Option<repriseapp::ReaderForm<'_>> {
        let s = self.sheet.as_mut().filter(|_| win != ERROR_WIN)?;
        Some(repriseapp::ReaderForm::Sheet(&mut s.sheet))
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let forms = ctx.forms();
        if win == ERROR_WIN {
            if let (Some(b), Some(text)) = (&self.error, &self.dialog_text) {
                b.access(tree, text, &forms);
            }
        } else if let Some(s) = &self.sheet {
            oneshot::access_sheet(&s.sheet, tree, &ctx.text, forms.fonts.size_px);
        }
    }
}

impl PowerApp {
    fn dispatch(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        if matches!(ev, Event::Wake) {
            return self.wake();
        }
        if win == ERROR_WIN {
            return self.error_event(ev);
        }
        self.ensure(ctx);
        let Some(s) = &mut self.sheet else {
            return Reply::none();
        };
        let out = match ev {
            Event::CloseRequest => {
                self.hidden = true;
                return Reply::redraw();
            }
            ev => match sheet_input(&mut s.sheet, &ev, ctx) {
                Some(ev) => s.event(ev),
                None => return Reply::none(),
            },
        };
        self.outcome(out)
    }
}

/// Runs the program; `args` are the command line after the program's name.
pub fn main(args: Vec<std::ffi::OsString>) {
    repriseapp::applet_main(&args, |tray_only| PowerApp {
        hidden: tray_only,
        tray_only,
        path: repriseconf::config_path(),
        ..PowerApp::default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use upower::Battery;

    /// An edit another writer made while the sheet stayed hidden reaches the alarms with the next battery snapshot.
    #[test]
    fn a_snapshot_takes_the_thresholds_the_file_has_now() {
        let path = std::env::temp_dir().join(format!("reprise-power-{}.toml", std::process::id()));
        std::fs::write(&path, "[power]\nbattery_low = 10\n").unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = PowerApp {
            path: path.clone(),
            snapshots: Some(rx),
            ..PowerApp::default()
        };
        app.reload_settings();
        std::fs::write(&path, "[power]\nbattery_low = 50\n").unwrap();
        tx.send(Power {
            on_battery: true,
            battery: Some(Battery {
                state: State::Discharging,
                percent: 40,
                ..Battery::default()
            }),
        })
        .unwrap();
        app.wake();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(app.settings.battery_low, 50);
        assert_ne!(app.alarms, alarms::Alarms::default(), "the low alarm fired");
    }

    /// A wake that shows no window leaves a second instance's handoff for the wake whose raise does.
    #[test]
    fn the_handoff_waits_for_the_wake_that_shows_the_window() {
        let handoff = reprisebus::instance::Handoff {
            token: Some("t".into()),
            by_key: true,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = PowerApp {
            tray: Some(reprisetray::TrayHandle::holding(handoff.clone())),
            msgs: Some(rx),
            hidden: true,
            ..PowerApp::default()
        };
        assert_eq!(app.wake().activate, None);
        tx.send(reprisetray::Rows::clicked(&tray::TrayMsg::OpenProperties))
            .unwrap();
        assert_eq!(app.wake().activate, Some((repriseapp::MAIN, handoff)));
    }
}
