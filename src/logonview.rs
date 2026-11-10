//! A login screen's host glue, shared by the greeter and the lock: keys, pointer, power menu, and
//! covers with the screen on one output and a backdrop on the rest.

use reprisekit::interface::Interface;
use reprisekit::interfaces::logon::{draw_logon, logon_column, logon_layout};
use reprisekit::logon::{self, Logon, LogonAction, LogonKey, LogonLayout, Part};
use reprisekit::pointer::Interaction;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::textedit::{EditKey, key_for};
use reprisekit::theme::Theme;
use reprisescene::{BufferId, RectId, TreeId};
use smithay::backend::input::InputTime;
use smithay::input::keyboard::{Keycode, KeysymHandle, ModifiersState, xkb};
use smithay::input::pointer::AxisFrame;
use smithay::output::Output;
use smithay::utils::{Logical, Point};

use crate::compose::{Compose, Fed};
use crate::compositor::State;
use crate::editkeys::KeyText;
use crate::keys;
use crate::menuview::{MenuFrom, MenuKind};
use crate::startmenu::{MenuItem, MenuStyle};

/// One host of a login screen: where its view lives (with the text renderer, borrowed apart from
/// it), what the screen's actions do, and the Caps Lock of the keys it reads.
pub struct Host {
    pub view: fn(&mut State) -> Option<(&mut LogonView, &TextRenderer)>,
    pub act: fn(&mut State, LogonAction),
    pub caps_lock: fn(&State) -> bool,
}

pub struct LogonView {
    pub model: Logon,
    pub covers: Vec<Cover>,
    /// The screen's parts on its output, in layout coordinates.
    pub layout: Option<LogonLayout>,
    pub compose: Option<Compose>,
    /// The screen is drawn but not shown, and takes no pointer.
    pub hidden: bool,
    /// The wheel's sub-notch remainder, in 120ths.
    wheel: i32,
}

impl LogonView {
    pub fn new(model: Logon, compose: Option<Compose>) -> LogonView {
        LogonView {
            model,
            covers: Vec::new(),
            layout: None,
            compose,
            hidden: false,
            wheel: 0,
        }
    }

    /// The part under layout point `pos`; none while hidden.
    pub fn part_at(&self, pos: Point<f64, Logical>) -> Option<Part> {
        let l = self.layout.as_ref().filter(|_| !self.hidden)?;
        logon::hit(l, pos.x.floor() as i32, pos.y.floor() as i32)
    }

    /// Whether `pos` lies on the screen's dialog or one of its parts; never while hidden.
    pub fn covers_point(&self, pos: Point<f64, Logical>) -> bool {
        let (x, y) = (pos.x.floor() as i32, pos.y.floor() as i32);
        self.part_at(pos).is_some()
            || (self.layout.as_ref())
                .filter(|_| !self.hidden)
                .and_then(|l| l.dialog)
                .is_some_and(|d| d.contains(x, y))
    }
}

/// One output's cover: the backdrop, a wall for a picture over it when asked for, and on the
/// screen's output the login screen on top.
pub struct Cover {
    pub output: Output,
    pub tree: TreeId,
    pub rect: RectId,
    pub wall: Option<BufferId>,
    pub screen: Option<BufferId>,
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

/// A press `h` decoded for the login screen, inside the keyboard's filter: only xkb and the
/// compose state are read, so nothing here reaches the keyboard's focus.
pub fn decode(state: &mut State, host: &Host, h: &KeysymHandle<'_>) -> Option<Typed> {
    let mods = (state.seat.get_keyboard()).map(|k| k.modifier_state())?;
    let guard = h.xkb().lock().ok()?;
    // SAFETY: the state reference does not outlive the lock guard.
    let xkb = unsafe { guard.state() };
    decode_with(state, host, &mods, xkb, h.raw_code())
}

/// The key `code` under `xkb` with `mods` held, decoded for the login screen.
pub fn decode_with(
    state: &mut State,
    host: &Host,
    mods: &ModifiersState,
    xkb: &xkb::State,
    code: Keycode,
) -> Option<Typed> {
    let (v, _) = (host.view)(state)?;
    let fed = match v.compose.as_mut() {
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
pub fn key(state: &mut State, host: &Host, typed: Option<Typed>) {
    let caps = (host.caps_lock)(state);
    let Some((v, _)) = (host.view)(state) else {
        return;
    };
    let Some(t) = typed else {
        if v.model.caps_lock != caps {
            redraw(state, host);
        }
        return;
    };
    let m = &mut v.model;
    let action = match &t {
        Typed::PowerKey if !m.offers_power() => None,
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
    act(state, host, action);
    redraw(state, host);
}

/// Text typed without a key (an input method, a remote keyboard), into the focused field.
pub fn type_text(state: &mut State, host: &Host, text: &str) {
    if text.chars().any(char::is_control) {
        return;
    }
    let Some((v, _)) = (host.view)(state) else {
        return;
    };
    let action = v.model.key(LogonKey::Text(text));
    act(state, host, action);
    redraw(state, host);
}

/// A held Backspace repeats; no other key does, so no typed character is kept for a repeat.
pub fn repeat_backspace(state: &mut State, host: &Host) {
    let Some((v, _)) = (host.view)(state) else {
        return;
    };
    v.model
        .key(LogonKey::Edit(EditKey::Backspace { word: false }));
    redraw(state, host);
}

/// A left press clicks the part under it.
pub fn click(state: &mut State, host: &Host, pos: Point<f64, Logical>) {
    let Some((v, _)) = (host.view)(state) else {
        return;
    };
    let Some(part) = v.part_at(pos) else {
        return;
    };
    let action = v.model.click(part);
    act(state, host, action);
    redraw(state, host);
}

/// The pointer moved over the login screen: no client has it, and the part under it lights.
pub fn motion(state: &mut State, host: &Host, pos: Point<f64, Logical>, time: InputTime) {
    state.pointer_motion(None, pos, time);
    state.set_cursor("default");
    let Some((v, _)) = (host.view)(state) else {
        return;
    };
    let lit = v.part_at(pos).map(|p| (p, Interaction::Hot));
    if v.model.lit != lit {
        v.model.lit = lit;
        redraw(state, host);
    }
}

/// The wheel over the user tiles steps through them, as Up and Down do.
pub fn wheel(state: &mut State, host: &Host, frame: &AxisFrame) {
    let pos = state.pointer_location();
    let Some((v, _)) = (host.view)(state) else {
        return;
    };
    let over = (v.layout.as_ref()).is_some_and(|l| {
        (l.parts.iter())
            .filter(|(p, _)| matches!(p, Part::Tile(_)))
            .any(|(_, r)| r.contains(pos.x as i32, pos.y as i32))
    });
    if !over {
        return;
    }
    let (steps, residual) =
        crate::pointers::wheel_steps(frame.v120.map_or(0, |v| v.1), frame.axis.1, v.wheel);
    v.wheel = residual;
    if steps == 0 {
        return;
    }
    let key = if steps > 0 {
        LogonKey::Up
    } else {
        LogonKey::Down
    };
    for _ in 0..steps.unsigned_abs() {
        v.model.key(key.clone());
    }
    redraw(state, host);
}

fn act(state: &mut State, host: &Host, action: Option<LogonAction>) {
    if let Some(action) = action {
        (host.act)(state, action);
    }
}

/// `items` over the screen's power arrow (or its power button), picked as `kind`.
pub fn open_power_menu(state: &mut State, host: &Host, items: Vec<MenuItem>, kind: MenuKind) {
    let Some((v, _)) = (host.view)(state) else {
        return;
    };
    let Some(output) = (v.covers.iter())
        .find(|c| c.screen.is_some())
        .map(|c| c.output.clone())
    else {
        return;
    };
    let anchor = (v.layout.as_ref()).and_then(|l| l.rect(Part::PowerMenu).or(l.rect(Part::Power)));
    let usable = state.layout_box(Some(&output));
    let anchor = anchor.unwrap_or(usable);
    let layout = MenuStyle::context_for(&state.config.theme, &items).layout_in(
        &state.config.theme,
        &state.text,
        &items,
        usable,
    );
    let x = (anchor.right() - layout.width).max(usable.x);
    let y = (anchor.y - layout.height).max(usable.y);
    state.push_menu(0, (x, y), layout, items, output, kind, MenuFrom::Elsewhere);
    state.redraw_menu(0);
}

/// The outputs with a layout box, left to right, then top to bottom.
pub fn shown_outputs(state: &State) -> Vec<Output> {
    let mut shown: Vec<(Output, Rect)> = (state.scene.outputs().iter())
        .map(|o| (o.clone(), state.layout_box(Some(o))))
        .filter(|(_, b)| b.width > 0)
        .collect();
    shown.sort_by_key(|(_, b)| (b.x, b.y));
    shown.into_iter().map(|(o, _)| o).collect()
}

/// The welcome backdrop: the desktop colour on Classic, the scheme's welcome field on Blue, and on
/// Glass none, since a picture shows there.
pub fn backdrop(state: &State) -> Option<[f32; 4]> {
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

/// Where a host's covers go: `outputs` each get one under `layer`, `color` behind (none leaves it
/// clear), a wall over it when `wall`, and the screen on `screen_on`.
pub struct Fit<'a> {
    pub layer: TreeId,
    pub outputs: Vec<Output>,
    pub screen_on: Option<&'a Output>,
    pub color: Option<[f32; 4]>,
    pub wall: bool,
}

/// The outputs a [`refit`] made covers for and those whose covers it took down.
#[derive(Default)]
pub struct Refit {
    pub added: Vec<Output>,
    pub removed: Vec<Output>,
}

/// Cover `fit.outputs` and only those, then draw the screen.
pub fn refit(state: &mut State, host: &Host, fit: Fit<'_>) -> Refit {
    let mut changed = Refit::default();
    let Some((v, _)) = (host.view)(state) else {
        return changed;
    };
    let mut covers = std::mem::take(&mut v.covers);
    covers.retain(|c| {
        let kept = fit.outputs.contains(&c.output);
        if !kept {
            state.scene.destroy(c.tree);
            changed.removed.push(c.output.clone());
        }
        kept
    });
    for output in &fit.outputs {
        let b = state.layout_box(Some(output));
        let at = match covers.iter().position(|c| c.output == *output) {
            Some(at) => at,
            None => {
                let tree = state.scene.tree_create(fit.layer);
                let rect = state.scene.rect_create(tree, (b.width, b.height), [0.0; 4]);
                covers.push(Cover {
                    output: output.clone(),
                    tree,
                    rect,
                    wall: None,
                    screen: None,
                });
                changed.added.push(output.clone());
                covers.len() - 1
            }
        };
        let c = &mut covers[at];
        state.scene.set_position(c.tree, b.x, b.y);
        state.scene.rect_set_size(c.rect, (b.width, b.height));
        (state.scene).rect_set_color(c.rect, fit.color.unwrap_or([0.0; 4]));
        state.scene.set_enabled(c.rect, fit.color.is_some());
        match (fit.wall, c.wall) {
            (true, None) => {
                let wall = state.scene.buffer_create(c.tree);
                if let Some(screen) = c.screen {
                    state.scene.place_below(wall, screen);
                }
                c.wall = Some(wall);
            }
            (false, Some(wall)) => {
                state.scene.destroy(wall);
                c.wall = None;
            }
            _ => {}
        }
        match (fit.screen_on == Some(output), c.screen) {
            (true, None) => c.screen = Some(state.scene.buffer_create(c.tree)),
            (false, Some(screen)) => {
                state.scene.destroy(screen);
                c.screen = None;
            }
            _ => {}
        }
    }
    if let Some((v, _)) = (host.view)(state) {
        v.covers = covers;
    }
    redraw(state, host);
    changed
}

/// Caps Lock of the seat's keyboard.
pub fn seat_caps_lock(state: &State) -> bool {
    (state.seat.get_keyboard()).is_some_and(|k| k.modifier_state().caps_lock)
}

#[cfg(test)]
thread_local! {
    /// The next draw of a login screen panics, as a painter's bug would.
    pub static DRAW_PANICS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Draw the login screen again with the Caps Lock and layout the keyboard has now.
pub fn redraw(state: &mut State, host: &Host) {
    let caps = (host.caps_lock)(state);
    let layout = state.layout_badge();
    let Some((v, _)) = (host.view)(state) else {
        return;
    };
    v.model.caps_lock = caps;
    v.model.layout = layout;
    let Some((output, buffer)) =
        (v.covers.iter()).find_map(|c| Some((c.output.clone(), c.screen?)))
    else {
        return;
    };
    let b = state.layout_box(Some(&output));
    let scale = state.draw_scale_at(Some(&output));
    let theme = state.config.theme.scaled(scale);
    let Some((v, text)) = (host.view)(state) else {
        return;
    };
    #[cfg(test)]
    if DRAW_PANICS.get() {
        panic!("draw_logon");
    }
    let (l, canvas) = screen(&mut v.model, &theme, text, b, scale);
    v.layout = Some(l);
    let shown = !v.hidden;
    crate::output::upload_scaled(&mut state.scene, buffer, &canvas, scale);
    state.scene.set_enabled(buffer, shown);
    state.redraw_all();
}

/// `m` laid out over `output` (layout coordinates) and drawn at `scale`, the canvas covering it.
pub fn screen(
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
