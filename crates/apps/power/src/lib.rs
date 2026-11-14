//! Power Options: the Reprise battery icon and power settings.
//!
//! One process: a tray icon that follows UPower's display device, the low and critical battery alarms,
//! and the Power Options Properties sheet behind it, which edits `config.toml`'s `[idle]`,
//! `[idle.battery]` and `[power]` keys, and the battery charge limit, which acts at once. The sheet and
//! the tray menu also switch power-profiles-daemon's profile, when the daemon is there, and the low battery
//! saver holds its power saver profile while the charge is low. `--tray` starts
//! with the sheet hidden (the autostart entry); plain `reprise-power` shows it. Closing the sheet hides
//! it; only Exit in the tray menu quits. The compositor does the acting: the timeouts, lid and buttons
//! are its, read from the same file.

#[cfg(test)]
reprisebus::test_session!();

mod alarms;
mod logind;
mod profiles;
mod settings;
mod sheet;
mod tray;
mod upower;

use std::time::{Duration, Instant};

use repriseapp::keys::sheet_input;
use repriseapp::oneshot::{self, MsgBox};
use repriseapp::{App, Child, Ctx, Event, Reply, WinId};
use reprisebus::charge::{self, ChargeLimit};
use reprisekit::access::Tree;
use reprisekit::art::powericon;
use reprisekit::controls::propsheet;
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError};

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
    saver: alarms::Saver,
    hold: Option<profiles::Hold>,
    /// The balloon waiting for the hold's answer.
    waiting: Option<Waiting>,
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
    /// The charge limit toggle under way.
    charging: Option<Receiver<Charged>>,
    waker: Option<repriseapp::Waker>,
}

/// A charge limit toggle's result: the limit as UPower tells it afterwards, and the toggle's
/// failure, `None` inside when the password prompt was dismissed.
struct Charged {
    limit: Option<ChargeLimit>,
    failed: Option<Option<String>>,
}

fn toggle(device: &str, on: bool) -> Charged {
    Charged {
        failed: charge::set(device, on).err(),
        // Tests leave the system bus alone.
        limit: if cfg!(test) { None } else { charge::read() },
    }
}

const ERROR_WIN: WinId = WinId::named("error");

/// The low battery balloon; `saving` adds that power saver is on.
/// How long a balloon waits for the power saver hold's answer; past it the low balloon goes out
/// without power saver.
const HOLD_WAIT: Duration = Duration::from_secs(3);

struct Waiting {
    /// The low alarm's percentage, or None for the saver's own balloon.
    low: Option<u8>,
    until: Instant,
}

fn low_balloon(pct: u8, saving: bool) -> (&'static str, String) {
    let saver = if saving {
        " The power saver profile is on until then."
    } else {
        ""
    };
    (
        "Low battery",
        format!("{pct}% of battery power remains. Plug the computer in.{saver}"),
    )
}

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
    /// `saving` says this snapshot asked for the power saver hold; the low balloon, or a balloon of
    /// its own, then waits for the answer ([`PowerApp::hold_answered`]).
    fn check_alarms(&mut self, saving: bool) -> Option<String> {
        let low = self.settings.battery_low.max(0) as u32;
        let critical = self.settings.battery_critical.max(0) as u32;
        match self.alarms.feed(&self.power, low, critical) {
            None => {
                if saving {
                    self.wait_for_hold(None);
                }
                None
            }
            Some(alarms::Alarm::Low(pct)) => {
                log::info!("low battery: {pct}%");
                if saving {
                    self.wait_for_hold(Some(pct));
                } else {
                    self.notify(low_balloon(pct, false));
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

    /// Take or let go of the power saver hold for the battery and profiles now known. True when taking it
    /// would move the profile, so a balloon says so once the hold answers.
    fn check_saver(&mut self) -> bool {
        let low = self.settings.battery_low.max(0) as u32;
        let on = self.settings.low_battery_saver
            && self.profiles.available.iter().any(|p| p == profiles::SAVER);
        let Some(hold) = self.saver.feed(&self.power, low, on) else {
            return false;
        };
        if !hold {
            self.waiting = None;
        }
        if let Some(h) = &self.hold
            && !h.set(hold)
            && hold
        {
            log::warn!("no profile hold thread; power saver stays off");
            self.saver = alarms::Saver::default();
            return false;
        }
        hold && self.profiles.active != profiles::SAVER
    }

    /// The hold thread answered a take. A refused hold is asked for again on the next snapshot; the
    /// balloon that waited goes out, naming power saver only when the hold took.
    fn hold_answered(&mut self, held: bool) -> Option<(&'static str, String)> {
        if !held {
            self.saver = alarms::Saver::default();
        }
        match self.waiting.take()?.low {
            Some(pct) => Some(low_balloon(pct, held)),
            None => held.then(|| {
                (
                    "Power saver on",
                    "The battery is low, so the power saver profile is on until the computer is plugged in."
                        .to_string(),
                )
            }),
        }
    }

    /// Hold a balloon for the hold's answer, waking the app when [`HOLD_WAIT`] ends.
    fn wait_for_hold(&mut self, low: Option<u8>) {
        self.waiting = Some(Waiting {
            low,
            until: Instant::now() + HOLD_WAIT,
        });
        if let Some(w) = self.waker.clone() {
            let spawned = std::thread::Builder::new()
                .name("reprise-power-wait".into())
                .spawn(move || {
                    std::thread::sleep(HOLD_WAIT);
                    w.wake();
                });
            if let Err(e) = spawned {
                log::warn!("could not start the hold wait: {e}");
            }
        }
    }

    /// The hold did not answer by `now`: the low balloon goes out without power saver, and the
    /// late answer shows nothing.
    fn hold_overdue(&mut self, now: Instant) -> Option<(&'static str, String)> {
        let low = self.waiting.as_ref().filter(|w| now >= w.until)?.low;
        self.waiting = None;
        low.map(|pct| low_balloon(pct, false))
    }

    fn notify(&self, (title, body): (&str, String)) {
        if let Some(t) = &self.tray {
            t.notify(title, &body);
        }
    }

    /// A restarted daemon dropped the saver's hold: let go of the dead one so the next check takes a new
    /// hold. A hold the user's pick had ended (the profile moved off power saver) stays ended.
    fn daemon_restarted(&mut self) {
        if self.saver == alarms::Saver::default() || self.profiles.active != profiles::SAVER {
            return;
        }
        if let Some(h) = &self.hold {
            h.set(false);
        }
        self.saver = alarms::Saver::default();
    }

    fn build(&mut self, icon: Option<Option<Canvas>>) {
        if self.sheet.is_some() && !(self.iconless && icon.is_some()) {
            return;
        }
        self.iconless = icon.is_none();
        let mut s = sheet::Sheet::new(&self.settings, &self.power, &self.profiles, icon.flatten());
        s.set_charging(self.charging.is_some());
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
            self.check_saver();
        } else {
            if let Some(s) = &mut self.sheet {
                s.unapplied(&self.settings);
            }
            self.fail(format!("{} could not be written.", self.path.display()));
        }
        ok
    }

    fn fail(&mut self, text: String) {
        self.error = Some(MsgBox::new(
            "Power Options",
            reprisekit::controls::msgbox::Fields {
                text,
                icon: None,
                buttons: vec!["OK".into()],
                default: 0,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        ));
    }

    /// Turn the charge limit on or off on a thread; a change while one runs shows the known limit again.
    fn charge(&mut self, device: String, on: bool) {
        if self.charging.is_some() {
            if let Some(s) = &mut self.sheet {
                s.charge_done(self.power.limit.clone());
            }
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let _ = tx.send(toggle(&device, on));
            if let Some(w) = waker {
                w.wake();
            }
        });
        self.charging = Some(rx);
    }

    /// A finished toggle: show the limit UPower reports and say why a refusal failed.
    fn charged(&mut self) {
        let done = match self.charging.as_ref().map(Receiver::try_recv) {
            None | Some(Err(TryRecvError::Empty)) => return,
            Some(Ok(c)) => c,
            Some(Err(TryRecvError::Disconnected)) => Charged {
                limit: self.power.limit.clone(),
                failed: None,
            },
        };
        self.charging = None;
        self.power.limit = done.limit;
        if let Some(s) = &mut self.sheet {
            s.charge_done(self.power.limit.clone());
        }
        if let Some(Some(why)) = done.failed {
            self.fail(format!("The charge limit could not be changed.\n\n{why}"));
        }
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
            sheet::Outcome::Charge { device, on } => {
                self.charge(device, on);
                Reply::redraw()
            }
        }
    }

    /// Drain the UPower, profile, charge limit and tray channels. Quits only on the tray's Exit.
    fn wake(&mut self) -> Reply {
        self.charged();
        for held in self
            .hold
            .as_ref()
            .map(profiles::Hold::answers)
            .unwrap_or_default()
        {
            if let Some(balloon) = self.hold_answered(held) {
                self.notify(balloon);
            }
        }
        if let Some(balloon) = self.hold_overdue(Instant::now()) {
            self.notify(balloon);
        }
        let fresh_profiles = self
            .profile_snapshots
            .as_ref()
            .and_then(|rx| rx.try_iter().last());
        let profiles_moved = fresh_profiles.is_some();
        if let Some(p) = fresh_profiles {
            if p.owner != self.profiles.owner {
                self.daemon_restarted();
            }
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
        let fresh_power = fresh.is_some();
        let mut spawn = None;
        if let Some(p) = fresh {
            self.power = p;
            if let Some(s) = &mut self.sheet {
                s.refresh_power(&self.power);
            }
        }
        if fresh_power || profiles_moved {
            // The alarms and the icon go by the file as it is now, other writers' edits included.
            self.reload_settings();
            self.push_state();
            let saving = self.check_saver();
            spawn = self.check_alarms(saving);
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
        self.hold = Some(profiles::Hold::start(waker.clone()));
        self.snapshots = Some(upower::watch(waker.clone()));
        self.waker = Some(waker);
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
    use reprisekit::controls::form;
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
            limit: None,
        })
        .unwrap();
        app.wake();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(app.settings.battery_low, 50);
        assert_ne!(app.alarms, alarms::Alarms::default(), "the low alarm fired");
    }

    #[test]
    fn the_saver_holds_once_the_daemon_offers_power_saver() {
        let low = Power {
            on_battery: true,
            battery: Some(Battery {
                state: State::Discharging,
                percent: 8,
                ..Battery::default()
            }),
            limit: None,
        };
        let offered = |active: &str| profiles::Profiles {
            available: vec!["power-saver".into(), "balanced".into()],
            active: active.into(),
            ..profiles::Profiles::default()
        };
        let mut app = PowerApp {
            power: low.clone(),
            ..PowerApp::default()
        };
        assert!(!app.check_saver(), "no daemon yet");
        app.profiles = offered("balanced");
        assert!(app.check_saver());
        assert!(!app.check_saver(), "once per crossing");
        let mut app = PowerApp {
            power: low.clone(),
            profiles: offered("power-saver"),
            ..PowerApp::default()
        };
        assert!(!app.check_saver(), "already on power saver");
        assert_ne!(app.saver, alarms::Saver::default(), "but held");
        let mut app = PowerApp {
            power: low,
            profiles: offered("balanced"),
            ..PowerApp::default()
        };
        app.settings.low_battery_saver = false;
        assert!(!app.check_saver());
        assert_eq!(app.saver, alarms::Saver::default());
    }

    #[test]
    fn a_hold_that_does_not_answer_lets_the_low_balloon_out_without_power_saver() {
        let mut app = PowerApp {
            power: Power {
                on_battery: true,
                battery: Some(Battery {
                    state: State::Discharging,
                    percent: 8,
                    ..Battery::default()
                }),
                limit: None,
            },
            profiles: profiles::Profiles {
                available: vec!["power-saver".into(), "balanced".into()],
                active: "balanced".into(),
                ..profiles::Profiles::default()
            },
            ..PowerApp::default()
        };
        app.settings.battery_low = 10;
        let saving = app.check_saver();
        assert_eq!(app.check_alarms(saving), None);
        let asked = Instant::now();
        assert_eq!(
            app.hold_overdue(asked),
            None,
            "the balloon waits for the answer"
        );
        assert_eq!(
            app.hold_overdue(asked + HOLD_WAIT),
            Some(low_balloon(8, false))
        );
        assert_eq!(app.hold_answered(true), None, "a late hold shows nothing");
        assert_ne!(app.saver, alarms::Saver::default(), "and keeps power saver");
    }

    #[test]
    fn a_refused_hold_is_asked_again_and_its_balloon_leaves_power_saver_out() {
        let mut app = PowerApp {
            power: Power {
                on_battery: true,
                battery: Some(Battery {
                    state: State::Discharging,
                    percent: 8,
                    ..Battery::default()
                }),
                limit: None,
            },
            profiles: profiles::Profiles {
                available: vec!["power-saver".into(), "balanced".into()],
                active: "balanced".into(),
                ..profiles::Profiles::default()
            },
            ..PowerApp::default()
        };
        app.settings.battery_low = 10;
        let saving = app.check_saver();
        assert!(saving);
        assert_eq!(app.check_alarms(saving), None);
        assert_eq!(
            app.waiting.as_ref().map(|w| w.low),
            Some(Some(8)),
            "the low balloon waits"
        );
        assert_eq!(
            app.hold_answered(false),
            Some(low_balloon(8, false)),
            "the low balloon, without power saver"
        );
        assert_eq!(app.saver, alarms::Saver::default(), "unheld");
        let saving = app.check_saver();
        assert!(saving, "the next snapshot asks again");
        assert_eq!(app.check_alarms(saving), None);
        assert_eq!(app.hold_answered(false), None, "no saver balloon");
        assert!(app.check_saver());
        app.check_alarms(true);
        let (title, _) = app.hold_answered(true).expect("a balloon");
        assert_eq!(title, "Power saver on");
        assert_ne!(app.saver, alarms::Saver::default(), "held");
    }

    #[test]
    fn a_restarted_daemon_is_held_again_unless_the_user_picked_meanwhile() {
        for (active, again) in [("power-saver", true), ("balanced", false)] {
            let mut app = PowerApp {
                power: Power {
                    on_battery: true,
                    battery: Some(Battery {
                        state: State::Discharging,
                        percent: 8,
                        ..Battery::default()
                    }),
                    limit: None,
                },
                profiles: profiles::Profiles {
                    available: vec!["power-saver".into(), "balanced".into()],
                    active: "balanced".into(),
                    owner: ":1.5".into(),
                },
                ..PowerApp::default()
            };
            assert!(app.check_saver());
            app.profiles.active = active.into();
            app.daemon_restarted();
            assert_eq!(app.saver == alarms::Saver::default(), again, "{active}");
            app.profiles.active = "balanced".into();
            assert_eq!(app.check_saver(), again, "{active}");
        }
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

    fn bat(enabled: bool) -> reprisebus::charge::ChargeLimit {
        reprisebus::charge::ChargeLimit {
            device: "/org/freedesktop/UPower/devices/battery_BAT0".into(),
            enabled,
            end: Some(80),
        }
    }

    fn toggled() -> (PowerApp, std::sync::mpsc::Sender<Charged>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = PowerApp {
            power: Power {
                limit: Some(bat(false)),
                ..Power::default()
            },
            ..PowerApp::default()
        };
        app.build(None);
        let s = app.sheet.as_mut().unwrap();
        s.open_tab(sheet::TAB_METER);
        s.sheet.pages[sheet::TAB_METER].set_check(sheet::CHARGE_LIMIT, true);
        let out = s.event(propsheet::Event::Page(
            sheet::TAB_METER,
            form::Event::Changed(sheet::CHARGE_LIMIT),
        ));
        assert!(matches!(out, sheet::Outcome::Charge { on: true, .. }));
        app.charging = Some(rx);
        (app, tx)
    }

    /// The charge limit check as (checked, enabled).
    fn check(app: &PowerApp) -> (bool, bool) {
        match &app.sheet.as_ref().unwrap().sheet.pages[sheet::TAB_METER]
            .item(sheet::CHARGE_LIMIT)
            .unwrap()
            .control
        {
            form::Control::Check {
                checked, enabled, ..
            } => (*checked, *enabled),
            c => panic!("a check box, not {c:?}"),
        }
    }

    #[test]
    fn a_refused_toggle_says_why_and_a_dismissed_prompt_says_nothing() {
        for (failed, said) in [
            (
                Some(Some("Not authorized".to_string())),
                Some("The charge limit could not be changed.\n\nNot authorized"),
            ),
            (Some(None), None),
        ] {
            let (mut app, tx) = toggled();
            assert_eq!(check(&app), (true, false));
            tx.send(Charged {
                limit: Some(bat(false)),
                failed,
            })
            .unwrap();
            app.wake();
            assert!(app.charging.is_none());
            assert_eq!(app.error.as_ref().map(|b| b.fields.text.as_str()), said);
            assert_eq!(check(&app), (false, true), "back at the known limit, live");
        }
    }

    #[test]
    fn a_rebuild_for_the_icon_keeps_a_running_toggle_grey() {
        let (mut app, _tx) = toggled();
        app.build(Some(None));
        assert!(!check(&app).1);
    }
}
