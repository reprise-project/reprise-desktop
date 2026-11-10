//! The login screen on the greeter's scene. Under `Host::Greeter` every key and press that no
//! open dialog or menu takes comes here, ahead of the binds (`keyroute`).

use reprisekit::interface::Interface;
use reprisekit::logon::{self, Logon, LogonAction, LogonUser, UserNote};
use repriselogin::greeter::{GreeterReply, Greeting, Note, Notice, PowerAction};
use smithay::backend::input::InputTime;
use smithay::input::keyboard::KeysymHandle;
use smithay::input::pointer::AxisFrame;
use smithay::utils::{Logical, Point};

use crate::compose::Compose;
use crate::compositor::State;
use crate::greeterconv::{Conversation, Step};
use crate::input::BTN_LEFT;
use crate::logonview::{self, Host, LogonView, Typed};
use crate::menuview::MenuKind;
use crate::startmenu::MenuItem;

#[derive(Default)]
pub struct GreeterView {
    /// The link to reprise-login closed; the greeter exits 1.
    pub lost: bool,
    pub login: Option<Greeter>,
    #[cfg(test)]
    pub heard: Vec<Heard>,
}

/// What reached the login screen, in order.
#[cfg(test)]
#[derive(Debug, PartialEq)]
pub enum Heard {
    Reply(GreeterReply),
    Key(Vec<u32>),
    Button(u32),
}

pub struct Greeter {
    pub view: LogonView,
    pub conv: Conversation,
    /// The session started; the loop stops after this turn.
    pub started: bool,
}

/// The greeter's covers lie in `layers.background`, under any dialog.
const HOST: Host = Host {
    view: |s| Some((&mut s.greeter.login.as_mut()?.view, &s.text)),
    act,
    caps_lock: logonview::seat_caps_lock,
};

fn user_note(note: Note) -> UserNote {
    match note {
        Note::LoggedOn => UserNote::LoggedOn,
        Note::Remote => UserNote::Remote,
        Note::Locked => UserNote::Locked,
    }
}

/// Starts the login screen from reprise-login's `greeting`.
pub fn setup(state: &mut State, g: Greeting) {
    let users = (g.users.iter().enumerate())
        .map(|(stock, u)| LogonUser {
            name: u.name.clone(),
            real_name: u.real_name.clone(),
            picture: (u.picture.as_deref()).and_then(crate::startpanelview::picture::load_picture),
            note: u.note.map(user_note),
            stock,
        })
        .collect();
    let interface = crate::greeter::interface(g.interface);
    let mut model = Logon::new(interface, users, g.show_users, g.last_user.as_deref());
    model.over_picture = crate::greeter::background(&g).is_some();
    if g.notice == Some(Notice::SessionEndedEarly) {
        model.info(logon::text::session_ended_early());
    }
    if g.notice == Some(Notice::SwitchUser) && g.show_users {
        model.pick_user();
    }
    state.greeter.login = Some(Greeter {
        view: LogonView::new(model, state.compose.as_ref().map(Compose::over)),
        conv: Conversation::default(),
        started: false,
    });
    refit(state);
}

/// One reply from reprise-login, in the order it sent them; none counts once the link is lost.
pub fn reply(state: &mut State, reply: GreeterReply) {
    #[cfg(test)]
    state.greeter.heard.push(Heard::Reply(reply.clone()));
    if state.greeter.lost {
        return;
    }
    let Some(g) = state.greeter.login.as_mut() else {
        return;
    };
    if let GreeterReply::Greeting(fresh) = &reply {
        if let Some(action) = regreet(g, fresh) {
            act(state, action);
        }
        return redraw(state);
    }
    for step in g.conv.on_reply(reply) {
        apply(state, step);
    }
    redraw(state);
}

/// A later Greeting: the running screen came forward, so its tiles' notes and the notice
/// catch up with the sessions, and Switch User's opens on the tiles.
fn regreet(g: &mut Greeter, fresh: &Greeting) -> Option<LogonAction> {
    for u in &mut g.view.model.users {
        let note = fresh
            .users
            .iter()
            .find(|f| f.name == u.name)
            .and_then(|f| f.note);
        u.note = note.map(user_note);
    }
    match fresh.notice {
        Some(Notice::SessionEndedEarly) => g.view.model.info(logon::text::session_ended_early()),
        Some(Notice::SwitchUser) if fresh.show_users => return g.view.model.pick_user(),
        _ => {}
    }
    None
}

fn apply(state: &mut State, step: Step) {
    if let Step::Send(req) = &step {
        crate::greeterlink::send(state, req);
        return;
    }
    let Some(g) = state.greeter.login.as_mut() else {
        return;
    };
    if step == Step::Started {
        g.started = true;
        state.signal.stop();
        return;
    }
    crate::greeterconv::show(&mut g.view.model, step);
}

/// The link to reprise-login closed or broke: nothing can log in through this greeter.
pub fn lost(state: &mut State) {
    log::error!("greeter: lost the link to the login manager");
    state.greeter.lost = true;
    state.signal.stop();
}

/// A press `h` decoded for the login screen, inside the keyboard's filter.
pub fn decode(state: &mut State, h: &KeysymHandle<'_>) -> Option<Typed> {
    #[cfg(test)]
    state.greeter.heard.push(Heard::Key(
        h.modified_syms().iter().map(|s| s.raw()).collect(),
    ));
    logonview::decode(state, &HOST, h)
}

pub fn key(state: &mut State, typed: Option<Typed>) {
    logonview::key(state, &HOST, typed);
}

pub fn type_text(state: &mut State, text: &str) {
    logonview::type_text(state, &HOST, text);
}

pub fn repeat_backspace(state: &mut State) {
    logonview::repeat_backspace(state, &HOST);
}

/// A press beside any dialog or menu: one closes an open menu, one beside an open dialog does
/// nothing, and otherwise a left press clicks the part under it.
pub fn button(state: &mut State, button: u32, pos: Point<f64, Logical>) {
    #[cfg(test)]
    state.greeter.heard.push(Heard::Button(button));
    if !state.menus.is_empty() {
        state.close_menu();
        return;
    }
    if button != BTN_LEFT || !state.dialogs.views.is_empty() {
        return;
    }
    logonview::click(state, &HOST, pos);
}

pub fn motion(state: &mut State, pos: Point<f64, Logical>, time: InputTime) {
    logonview::motion(state, &HOST, pos, time);
}

pub fn wheel(state: &mut State, frame: &AxisFrame) {
    logonview::wheel(state, &HOST, frame);
}

/// Turn Off or Restart, from the Turn Off dialog or Glass's power button and menu.
pub fn power(state: &mut State, action: PowerAction) {
    let Some(g) = state.greeter.login.as_mut() else {
        return;
    };
    for req in g.conv.power(action) {
        crate::greeterlink::send(state, &req);
    }
}

fn act(state: &mut State, action: LogonAction) {
    let Some(g) = state.greeter.login.as_mut() else {
        return;
    };
    match action {
        LogonAction::Submit { user } => {
            if let Some(req) = g.conv.on_submit(&mut g.view.model, user) {
                crate::greeterlink::send(state, &req);
            }
        }
        LogonAction::Cancel => {
            if let Some(req) = g.conv.cancel() {
                crate::greeterlink::send(state, &req);
            }
        }
        LogonAction::Power if g.view.model.interface == Interface::Glass => {
            power(state, PowerAction::PowerOff);
        }
        LogonAction::Power => state.open_dialog(crate::dialog::Dialog::greeter_turn_off()),
        LogonAction::PowerMenu => open_power_menu(state),
        LogonAction::NextLayout => state.next_layout(),
        LogonAction::SwitchUser => {}
    }
}

/// Restart and Shut down, over the screen's power arrow.
fn open_power_menu(state: &mut State) {
    let items = vec![
        MenuItem::item(&logon::text::restart()),
        MenuItem::item(&logon::text::power_off()),
    ];
    let kind = MenuKind::GreeterPower(vec![PowerAction::Reboot, PowerAction::PowerOff]);
    logonview::open_power_menu(state, &HOST, items, kind);
}

/// Cover every output, clear over a background or Glass's wallpaper, with the screen on the
/// greeting's primary one while shown, else the first in layout order. Runs on layout changes.
pub fn refit(state: &mut State) {
    let outputs = logonview::shown_outputs(state);
    let primary = state.config.primary_output_name();
    let screen_on = (outputs.iter())
        .find(|o| Some(o.name().as_str()) == primary)
        .or(outputs.first());
    let over_picture = (state.greeter.login.as_ref()).is_some_and(|g| g.view.model.over_picture);
    let fit = logonview::Fit {
        layer: state.layers.background,
        screen_on,
        color: (!over_picture)
            .then(|| logonview::backdrop(state))
            .flatten(),
        wall: false,
        outputs: outputs.clone(),
    };
    logonview::refit(state, &HOST, fit);
}

/// The greeting's background did not load: the login screen takes its interface's own look.
pub fn background_failed(state: &mut State) {
    let Some(g) = state.greeter.login.as_mut() else {
        return;
    };
    if !g.view.model.over_picture {
        return;
    }
    log::warn!("greeter: the background did not load; showing the default");
    g.view.model.over_picture = false;
    let fallback = crate::greeter::default_wallpaper(state.config.interface);
    state.config.wallpaper.image = fallback.clone();
    state.set_wallpaper(fallback.as_deref());
    refit(state);
}

pub fn redraw(state: &mut State) {
    logonview::redraw(state, &HOST);
}

#[cfg(test)]
mod tests;
