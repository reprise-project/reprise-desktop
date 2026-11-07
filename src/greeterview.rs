//! The login screen on the greeter's scene. Under `Host::Greeter` every key and press that no
//! open dialog or menu takes comes here, ahead of the binds (`keyroute`).

use reprisekit::interface::Interface;
use reprisekit::interfaces::logon::{draw_logon, logon_column, logon_layout};
use reprisekit::logon::{
    self, Logon, LogonAction, LogonKey, LogonLayout, LogonUser, Part, UserNote,
};
use reprisekit::pointer::Interaction;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::textedit::{EditKey, key_for};
use reprisekit::theme::Theme;
use repriselogin::greeter::{ErrorKind, GreeterReply, Greeting, Note, Notice, PowerAction};
use reprisescene::{BufferId, RectId, TreeId};
use smithay::backend::input::InputTime;
use smithay::input::keyboard::KeysymHandle;
use smithay::input::pointer::AxisFrame;
use smithay::output::Output;
use smithay::utils::{Logical, Point};
use zeroize::Zeroizing;

use crate::compose::{Compose, Fed};
use crate::compositor::State;
use crate::editkeys::KeyText;
use crate::greeterconv::{Conversation, Step};
use crate::input::BTN_LEFT;
use crate::keys;
use crate::menuview::{MenuFrom, MenuKind};
use crate::startmenu::{MenuItem, MenuStyle};

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
    pub model: Logon,
    pub conv: Conversation,
    covers: Vec<Cover>,
    /// The screen's parts on its output, in layout coordinates.
    layout: Option<LogonLayout>,
    compose: Option<Compose>,
    /// The wheel's sub-notch remainder, in 120ths.
    wheel: i32,
    /// The session started; the loop stops after this turn.
    pub started: bool,
}

/// One output's cover in `layers.background`: the interface's backdrop, and on the first output
/// the login screen.
struct Cover {
    output: Output,
    tree: TreeId,
    rect: RectId,
    screen: Option<BufferId>,
}

/// A press decoded for the login screen; typed text stays in a buffer zeroed on drop.
pub enum Typed {
    Enter,
    Escape,
    Tab {
        back: bool,
    },
    Up,
    Down,
    /// Never `EditKey::Insert`: text comes as `Text`.
    Edit(EditKey),
    Text(KeyText),
    PowerKey,
}

/// Starts the login screen from reprise-login's `greeting`.
pub fn setup(state: &mut State, g: Greeting) {
    let users = (g.users.iter())
        .map(|u| LogonUser {
            name: u.name.clone(),
            real_name: u.real_name.clone(),
            picture: (u.picture.as_deref()).and_then(crate::startpanelview::picture::load_picture),
            note: u.note.map(|Note::LoggedOn| UserNote::LoggedOn),
        })
        .collect();
    let interface = crate::greeter::interface(g.interface);
    let mut model = Logon::new(interface, users, g.show_users, g.last_user.as_deref());
    if g.notice == Some(Notice::SessionEndedEarly) {
        model.info(logon::text::session_ended_early());
    }
    state.greeter.login = Some(Greeter {
        model,
        conv: Conversation::default(),
        covers: Vec::new(),
        layout: None,
        compose: state.compose.as_ref().map(Compose::over),
        wheel: 0,
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
    for step in g.conv.on_reply(reply) {
        apply(state, step);
    }
    redraw(state);
}

fn apply(state: &mut State, step: Step) {
    if let Step::Send(req) = &step {
        crate::greeterlink::send(state, req);
        return;
    }
    let Some(g) = state.greeter.login.as_mut() else {
        return;
    };
    match step {
        Step::Send(_) => {}
        Step::Ask(prompt) => g.model.ask(prompt),
        Step::Note(text) => g.model.info(text),
        Step::Fail(kind, text) => g.model.fail(match kind {
            ErrorKind::Auth => logon::text::incorrect(),
            ErrorKind::Refused => logon::text::power_refused(),
            ErrorKind::Failed => text,
        }),
        Step::Idle => g.model.set_busy(false),
        Step::Started => {
            g.started = true;
            state.signal.stop();
        }
    }
}

/// The link to reprise-login closed or broke: nothing can log in through this greeter.
pub fn lost(state: &mut State) {
    log::error!("greeter: lost the link to the login manager");
    state.greeter.lost = true;
    state.signal.stop();
}

/// A press `h` decoded for the login screen, inside the keyboard's filter: only xkb and the
/// compose state are read, so nothing here reaches the keyboard's focus.
pub fn decode(state: &mut State, h: &KeysymHandle<'_>) -> Option<Typed> {
    #[cfg(test)]
    state.greeter.heard.push(Heard::Key(
        h.modified_syms().iter().map(|s| s.raw()).collect(),
    ));
    let mods = (state.seat.get_keyboard()).map(|k| k.modifier_state())?;
    let g = state.greeter.login.as_mut()?;
    let guard = h.xkb().lock().ok()?;
    // SAFETY: the state reference does not outlive the lock guard.
    let xkb = unsafe { guard.state() };
    let code = h.raw_code();
    let fed = match g.compose.as_mut() {
        Some(c) => c.feed(xkb.key_get_one_sym(code).raw()),
        None => Fed::Plain,
    };
    match fed {
        Fed::Swallowed => return None,
        Fed::Composed(t) => return Some(Typed::Text(t)),
        Fed::Plain => {}
    }
    for sym in xkb.key_get_syms(code) {
        return Some(match sym.raw() {
            keys::KEY_Return | keys::KEY_KP_Enter => Typed::Enter,
            keys::KEY_Escape => Typed::Escape,
            keys::KEY_Tab => Typed::Tab { back: mods.shift },
            keys::KEY_ISO_Left_Tab => Typed::Tab { back: true },
            keys::KEY_Up | keys::KEY_KP_Up => Typed::Up,
            keys::KEY_Down | keys::KEY_KP_Down => Typed::Down,
            keys::KEY_XF86PowerOff => Typed::PowerKey,
            s => match key_for(s, mods.ctrl, mods.shift, None) {
                Some(k) => Typed::Edit(k),
                None => continue,
            },
        });
    }
    if mods.ctrl || mods.logo {
        return None;
    }
    let t = KeyText::of_key(xkb, code)?;
    (!t.as_str().chars().any(char::is_control)).then_some(Typed::Text(t))
}

/// A key on the login screen, pressed (`typed`, from [`decode`]) or released: Caps Lock is
/// followed either way. The power key asks, as the screen's own power button does.
pub fn key(state: &mut State, typed: Option<Typed>) {
    let caps = caps_lock(state);
    let Some(g) = state.greeter.login.as_mut() else {
        return;
    };
    let Some(t) = typed else {
        if g.model.caps_lock != caps {
            redraw(state);
        }
        return;
    };
    let m = &mut g.model;
    let action = match &t {
        Typed::PowerKey => Some(match m.interface {
            Interface::Glass => LogonAction::PowerMenu,
            _ => LogonAction::Power,
        }),
        Typed::Enter => m.key(LogonKey::Enter),
        Typed::Escape => m.key(LogonKey::Escape),
        Typed::Tab { back } => m.key(LogonKey::Tab { back: *back }),
        Typed::Up => m.key(LogonKey::Up),
        Typed::Down => m.key(LogonKey::Down),
        Typed::Edit(k) => m.key(LogonKey::Edit(k.clone())),
        Typed::Text(text) => m.key(LogonKey::Text(text.as_str())),
    };
    act(state, action);
    redraw(state);
}

/// Text typed without a key (an input method, a remote keyboard), into the focused field.
pub fn type_text(state: &mut State, text: &str) {
    if text.chars().any(char::is_control) {
        return;
    }
    let Some(g) = state.greeter.login.as_mut() else {
        return;
    };
    let action = g.model.key(LogonKey::Text(text));
    act(state, action);
    redraw(state);
}

/// A held Backspace repeats; no other key does, so no typed character is kept for a repeat.
pub fn repeat_backspace(state: &mut State) {
    let Some(g) = state.greeter.login.as_mut() else {
        return;
    };
    g.model
        .key(LogonKey::Edit(EditKey::Backspace { word: false }));
    redraw(state);
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
    let Some(g) = state.greeter.login.as_mut() else {
        return;
    };
    let Some(part) = (g.layout.as_ref()).and_then(|l| logon::hit(l, pos.x as i32, pos.y as i32))
    else {
        return;
    };
    let action = g.model.click(part);
    act(state, action);
    redraw(state);
}

/// The pointer moved over the login screen: no client has it, and the part under it lights.
pub fn motion(state: &mut State, pos: Point<f64, Logical>, time: InputTime) {
    state.pointer_motion(None, pos, time);
    state.set_cursor("default");
    let Some(g) = state.greeter.login.as_mut() else {
        return;
    };
    let part = (g.layout.as_ref()).and_then(|l| logon::hit(l, pos.x as i32, pos.y as i32));
    let lit = part.map(|p| (p, Interaction::Hot));
    if g.model.lit != lit {
        g.model.lit = lit;
        redraw(state);
    }
}

/// The wheel over the user tiles steps through them, as Up and Down do.
pub fn wheel(state: &mut State, frame: &AxisFrame) {
    let pos = state.pointer_location();
    let Some(g) = state.greeter.login.as_mut() else {
        return;
    };
    let over = (g.layout.as_ref()).is_some_and(|l| {
        (l.parts.iter())
            .filter(|(p, _)| matches!(p, Part::Tile(_)))
            .any(|(_, r)| r.contains(pos.x as i32, pos.y as i32))
    });
    if !over {
        return;
    }
    let (steps, residual) =
        crate::pointers::wheel_steps(frame.v120.map_or(0, |v| v.1), frame.axis.1, g.wheel);
    g.wheel = residual;
    if steps == 0 {
        return;
    }
    let key = if steps > 0 {
        LogonKey::Up
    } else {
        LogonKey::Down
    };
    for _ in 0..steps.unsigned_abs() {
        g.model.key(key.clone());
    }
    redraw(state);
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

fn act(state: &mut State, action: Option<LogonAction>) {
    let Some(g) = state.greeter.login.as_mut() else {
        return;
    };
    let Some(action) = action else {
        return;
    };
    match action {
        LogonAction::Submit { user } => {
            let text = Zeroizing::new(g.model.password.text.clone());
            let req = if g.model.prompt.is_some() {
                g.conv.answer(text)
            } else if g.conv.idle() {
                Some(g.conv.submit(user, text))
            } else {
                None
            };
            let Some(req) = req else {
                return;
            };
            g.model.password.wipe();
            g.model.set_busy(true);
            crate::greeterlink::send(state, &req);
        }
        LogonAction::Cancel => {
            if let Some(req) = g.conv.cancel() {
                crate::greeterlink::send(state, &req);
            }
        }
        LogonAction::Power if g.model.interface == Interface::Glass => {
            power(state, PowerAction::PowerOff);
        }
        LogonAction::Power => state.open_dialog(crate::dialog::Dialog::greeter_turn_off()),
        LogonAction::PowerMenu => open_power_menu(state),
        LogonAction::NextLayout => state.next_layout(),
    }
}

/// Restart and Shut down, over the screen's power arrow.
fn open_power_menu(state: &mut State) {
    let Some(g) = state.greeter.login.as_ref() else {
        return;
    };
    let Some(output) = (g.covers.iter())
        .find(|c| c.screen.is_some())
        .map(|c| c.output.clone())
    else {
        return;
    };
    let usable = state.layout_box(Some(&output));
    let anchor = (g.layout.as_ref())
        .and_then(|l| l.rect(Part::PowerMenu).or(l.rect(Part::Power)))
        .unwrap_or(usable);
    let items = vec![
        MenuItem::item(&logon::text::restart()),
        MenuItem::item(&logon::text::power_off()),
    ];
    let layout = MenuStyle::context_for(&state.config.theme, &items).layout_in(
        &state.config.theme,
        &state.text,
        &items,
        usable,
    );
    let x = (anchor.right() - layout.width).max(usable.x);
    let y = (anchor.y - layout.height).max(usable.y);
    state.push_menu(
        0,
        (x, y),
        layout,
        items,
        output,
        MenuKind::GreeterPower(vec![PowerAction::Reboot, PowerAction::PowerOff]),
        MenuFrom::Elsewhere,
    );
    state.redraw_menu(0);
}

/// Cover every output and put the screen on the first in layout order (left to right, then
/// top to bottom), after the login screen starts and whenever the layout changes.
pub fn refit(state: &mut State) {
    let mut shown: Vec<(Output, Rect)> = (state.scene.outputs().iter())
        .map(|o| (o.clone(), state.layout_box(Some(o))))
        .filter(|(_, b)| b.width > 0)
        .collect();
    shown.sort_by_key(|(_, b)| (b.x, b.y));
    let color = cover_color(state);
    let Some(g) = state.greeter.login.as_mut() else {
        return;
    };
    let mut covers = std::mem::take(&mut g.covers);
    covers.retain(|c| {
        let kept = shown.iter().any(|(o, _)| *o == c.output);
        if !kept {
            state.scene.destroy(c.tree);
        }
        kept
    });
    for (i, (output, b)) in shown.iter().enumerate() {
        let at = match covers.iter().position(|c| c.output == *output) {
            Some(at) => at,
            None => {
                let tree = state.scene.tree_create(state.layers.background);
                let rect = state.scene.rect_create(tree, (b.width, b.height), [0.0; 4]);
                covers.push(Cover {
                    output: output.clone(),
                    tree,
                    rect,
                    screen: None,
                });
                covers.len() - 1
            }
        };
        let c = &mut covers[at];
        state.scene.set_position(c.tree, b.x, b.y);
        state.scene.rect_set_size(c.rect, (b.width, b.height));
        state
            .scene
            .rect_set_color(c.rect, color.unwrap_or([0.0; 4]));
        state.scene.set_enabled(c.rect, color.is_some());
        match (i == 0, c.screen) {
            (true, None) => c.screen = Some(state.scene.buffer_create(c.tree)),
            (false, Some(screen)) => {
                state.scene.destroy(screen);
                c.screen = None;
            }
            _ => {}
        }
    }
    if let Some(g) = state.greeter.login.as_mut() {
        g.covers = covers;
    }
    redraw(state);
}

/// The backdrop behind the screen: the desktop colour on Classic, the scheme's welcome field on
/// Blue, and on Glass none, since its wallpaper lies under the cover (`greeter::config_from`).
fn cover_color(state: &State) -> Option<[f32; 4]> {
    let theme = &state.config.theme;
    match state.config.interface {
        Interface::Classic => Some(theme.colors.background.as_f32()),
        Interface::Blue => {
            let scheme = reprisekit::blue::Variant::of(&theme.look).scheme();
            Some(scheme.welcome_field.as_f32())
        }
        Interface::Glass => None,
    }
}

fn caps_lock(state: &State) -> bool {
    (state.seat.get_keyboard()).is_some_and(|k| k.modifier_state().caps_lock)
}

/// Draw the login screen again with the Caps Lock and layout the keyboard has now.
pub fn redraw(state: &mut State) {
    let caps = caps_lock(state);
    let layout = state.layout_badge();
    let Some(g) = state.greeter.login.as_mut() else {
        return;
    };
    g.model.caps_lock = caps;
    g.model.layout = layout;
    let Some((output, buffer)) =
        (g.covers.iter()).find_map(|c| Some((c.output.clone(), c.screen?)))
    else {
        return;
    };
    let b = state.layout_box(Some(&output));
    let scale = state.draw_scale_at(Some(&output));
    let theme = state.config.theme.scaled(scale);
    let Some(g) = state.greeter.login.as_mut() else {
        return;
    };
    let (l, canvas) = screen(&mut g.model, &theme, &state.text, b, scale);
    g.layout = Some(l);
    crate::output::upload_scaled(&mut state.scene, buffer, &canvas, scale);
    state.redraw_all();
}

/// `m` laid out over `output` (layout coordinates) and drawn at `scale`, the canvas covering it.
fn screen(
    m: &mut Logon,
    theme: &Theme,
    text: &TextRenderer,
    output: Rect,
    scale: i32,
) -> (LogonLayout, Canvas) {
    m.column = logon_column(m, theme, output);
    let l = logon_layout(m, theme, text, output);
    let mut c = Canvas::new(
        (output.width * scale) as u32,
        (output.height * scale) as u32,
    );
    draw_logon(&mut c, m, &l, theme, text.for_output(scale), scale);
    (l, c)
}

#[cfg(test)]
mod tests;
