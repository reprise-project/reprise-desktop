//! Screen reader side of the compositor's own surfaces: one AccessKit adapter per taskbar, menu stack, switcher, dialog, launcher, clipboard history, emoji picker, calendar, displays panel, tray flyout, task preview panel, notification history and the balloons, speaking AT-SPI on the session's accessibility bus.
//! `accesskit_unix` runs its D-Bus side on its own thread and calls the handlers (`reprisekit::access::Handler`) there; they post a `Msg` to a calloop channel, so the loop never waits on the bus. Trees are built on the loop, in an idle pass [`touch`] queues, only for adapters a reader holds.
//! Bounds are layout coordinates: a tree's root carries its surface's place as a transform and its controls lay out surface-local. A tree claims the focus only while its surface has the keyboard (see [`focus_of`]). The lock screen has no tree, so its password field's text stays here.
//! A reader's action on a node runs as the click a user would give it ([`click`], `reprisekit::access::gesture`) through the pointer's press and release paths; focusing a taskbar item gives the bar the keyboard (`barkeys`).
//! The same idle pass moves the seat's keyboard for the menus, Glass Start menu, launcher and overview, reader or not: they take it from the window while open and give it back on close ([`move_keyboard`]).

use reprisebus::a11y::{KeyEcho, KeyNote};
use reprisekit::access::accesskit::{Affine, Role};
use reprisekit::access::{Act, Gesture, Handler, Published, Request, Tree};
use reprisescene::TreeId;
use smithay::input::keyboard::Keycode;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::calloop::channel::{self, Event, Sender};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use zeroize::Zeroizing;

use crate::compositor::State;
use crate::input::{Hit, Interaction};
use crate::keyroute::KeyPress;
use crate::raster::Rect;
use smithay::backend::input::InputTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Surface {
    Taskbar(TreeId),
    Menus,
    Switcher,
    Overview,
    Dialog(u64),
    Launcher,
    StartPanel,
    ClipView,
    EmojiView,
    Calendar,
    Displays,
    TrayFlyout,
    SnapPicker,
    TaskPreview,
    /// Always there (see `notifyview::access`), so a balloon's alert is announced.
    Balloons,
    NotifyCenter,
}

type Msg = reprisekit::access::Msg<Surface>;

struct Slot {
    surface: Surface,
    adapter: accesskit_unix::Adapter,
    published: Published,
    active: bool,
    focused: bool,
}

impl Slot {
    fn new(surface: Surface, tx: &Sender<Msg>) -> Slot {
        let handler = || {
            let tx = tx.clone();
            Handler::new(surface, move |m| {
                let _ = tx.send(m);
            })
        };
        Slot {
            surface,
            adapter: accesskit_unix::Adapter::new(handler(), handler(), handler()),
            published: Published::default(),
            active: false,
            focused: false,
        }
    }
}

#[derive(Default)]
pub struct Access {
    slots: Vec<Slot>,
    /// None when the channel could not join the loop: nothing is ever
    /// published.
    tx: Option<Sender<Msg>>,
    queued: bool,
    keys: Option<KeyEcho>,
    /// The surface a popup took the seat's keyboard from (see
    /// [`move_keyboard`]); it goes back when the popup lets it go.
    lent: Option<WlSurface>,
    /// Where a click the compositor makes itself ([`click`]) lands, while
    /// it runs: `State::pointer_location` reads this instead of the
    /// pointer.
    pub click_at: Option<Point<f64, Logical>>,
}

impl Access {
    pub fn new(handle: &LoopHandle<'static, State>) -> Access {
        let (tx, rx) = channel::channel::<Msg>();
        let joined = handle.insert_source(rx, |ev, _, state| {
            if let Event::Msg(msg) = ev {
                let (surface, on) = match msg {
                    Msg::Activate(s) => (s, true),
                    Msg::Deactivate(s) => (s, false),
                    Msg::Act(s, req) => {
                        act(state, s, req);
                        touch(state);
                        return;
                    }
                };
                if let Some(slot) = state.access.slots.iter_mut().find(|x| x.surface == surface) {
                    slot.published.reset();
                    slot.active = on;
                    slot.focused = false;
                }
                touch(state);
            }
        });
        if let Err(e) = joined {
            log::warn!(
                "access: {}; screen readers get no compositor trees",
                e.error
            );
            return Access::default();
        }
        Access {
            tx: Some(tx),
            ..Access::default()
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keyboard {
    /// The lock, an input capture, the region overlay or a keyboard move
    /// holds it: none of the trees.
    pub elsewhere: bool,
    pub switcher: bool,
    pub overview: bool,
    pub snap_picker: bool,
    pub menus: bool,
    pub launcher: bool,
    pub start_panel: bool,
    pub clipview: bool,
    pub emojiview: bool,
    pub notify_reply: bool,
    pub notify_center: bool,
    pub displays: bool,
    pub taskbar: Option<TreeId>,
    pub dialog: Option<u64>,
    pub calendar: bool,
    pub client: bool,
}

/// The surface whose tree has the focus, in `keyroute::route`'s order.
/// The calendar takes no keys but Escape, so it claims the focus only
/// while no client has the keyboard.
pub fn focus_of(k: &Keyboard) -> Option<Surface> {
    if k.elsewhere {
        None
    } else if k.switcher {
        Some(Surface::Switcher)
    } else if k.overview {
        Some(Surface::Overview)
    } else if k.snap_picker {
        Some(Surface::SnapPicker)
    } else if k.menus {
        Some(Surface::Menus)
    } else if k.start_panel {
        Some(Surface::StartPanel)
    } else if k.displays {
        Some(Surface::Displays)
    } else if k.launcher {
        Some(Surface::Launcher)
    } else if k.clipview {
        Some(Surface::ClipView)
    } else if k.emojiview {
        Some(Surface::EmojiView)
    } else if k.notify_reply {
        Some(Surface::Balloons)
    } else if k.notify_center {
        Some(Surface::NotifyCenter)
    } else if let Some(t) = k.taskbar {
        Some(Surface::Taskbar(t))
    } else if let Some(id) = k.dialog {
        Some(Surface::Dialog(id))
    } else if k.calendar && !k.client {
        Some(Surface::Calendar)
    } else {
        None
    }
}

fn keyboard(state: &State) -> Keyboard {
    Keyboard {
        elsewhere: state.is_locked()
            || state.inputproto.gated()
            || state.panels.region.is_some()
            || state.interaction.is_keyboard(),
        switcher: state.switcher.is_open(),
        overview: state.overview.view.is_some(),
        snap_picker: state.snap_picker.has_keys(),
        menus: !state.menus.is_empty(),
        launcher: state.launcher.is_some(),
        start_panel: state.start_panel.is_some(),
        clipview: state.panels.clipview.is_some(),
        emojiview: state.panels.emojiview.is_some(),
        notify_reply: crate::notifyview::reply_focused(state),
        notify_center: crate::notifycenter::is_open(state),
        displays: crate::displaysview::is_open(state),
        taskbar: state.bar_shared.keys.map(|k| k.bar),
        dialog: state.active_dialog_id(),
        calendar: crate::calendarview::is_open(state),
        client: state.focused.is_some() || state.layers_shell.focused.is_some(),
    }
}

/// The focused control of the compositor's surface that has the
/// keyboard, in layout coordinates: what a zoomed view holds
/// (`magnifier`). Builds that one tree, reader or not.
pub fn focus_rect(state: &State) -> Option<Rect> {
    let surface = focus_of(&keyboard(state))?;
    let b = tree(state, surface)?.focus_bounds()?;
    let (x0, y0) = (b.x0.floor() as i32, b.y0.floor() as i32);
    Some(Rect::new(
        x0,
        y0,
        b.x1.ceil() as i32 - x0,
        b.y1.ceil() as i32 - y0,
    ))
}

pub fn root(role: Role, name: &str, rect: Rect) -> Tree {
    let mut n = reprisekit::access::node(role, Rect::new(0, 0, rect.width, rect.height));
    if !name.is_empty() {
        n.set_label(name);
    }
    n.set_transform(Affine::translate((f64::from(rect.x), f64::from(rect.y))));
    Tree::new(n)
}

pub fn touch(state: &mut State) {
    let a = &mut state.access;
    if a.queued {
        return;
    }
    a.queued = true;
    state.handle.insert_idle(|state| {
        state.access.queued = false;
        sync(state);
    });
}

fn surfaces(state: &State) -> Vec<Surface> {
    let mut v: Vec<Surface> = state
        .bars
        .iter()
        .filter(|b| state.layout_box(Some(&b.output)).width > 0)
        .map(|b| Surface::Taskbar(b.tree))
        .collect();
    let open = [
        (!state.menus.is_empty(), Surface::Menus),
        (state.switcher.is_open(), Surface::Switcher),
        (state.overview.view.is_some(), Surface::Overview),
        (state.launcher.is_some(), Surface::Launcher),
        (state.start_panel.is_some(), Surface::StartPanel),
        (state.panels.clipview.is_some(), Surface::ClipView),
        (state.panels.emojiview.is_some(), Surface::EmojiView),
        (crate::calendarview::is_open(state), Surface::Calendar),
        (crate::displaysview::is_open(state), Surface::Displays),
        (
            crate::trayflyout::open_on(state).is_some(),
            Surface::TrayFlyout,
        ),
        (state.task_preview.is_shown(), Surface::TaskPreview),
        (state.snap_picker.is_shown(), Surface::SnapPicker),
        (crate::notifycenter::is_open(state), Surface::NotifyCenter),
        (true, Surface::Balloons),
    ];
    v.extend(open.into_iter().filter_map(|(on, x)| on.then_some(x)));
    v.extend(
        (state.dialogs.views.iter())
            .filter(|d| !d.minimized)
            .map(|d| Surface::Dialog(d.id)),
    );
    v
}

fn tree(state: &State, surface: Surface) -> Option<Tree> {
    match surface {
        Surface::Taskbar(t) => state.bar(t).map(|b| state.taskbar_access(b)),
        Surface::Menus => state.menus_access(),
        Surface::Switcher => state.switcher_access(),
        Surface::Overview => state.overview_access(),
        Surface::Dialog(id) => state.dialog_access(id),
        Surface::Launcher => state.launcher_access(),
        Surface::StartPanel => state.start_panel_access(),
        Surface::TrayFlyout => crate::trayflyout::access(state),
        Surface::TaskPreview => crate::taskpreview::access(state),
        Surface::SnapPicker => crate::snappickerview::access(state),
        Surface::ClipView => state.clipview_access(),
        Surface::EmojiView => state.emoji_access(),
        Surface::Calendar => crate::calendarview::access(state),
        Surface::Displays => crate::displaysview::access(state),
        Surface::NotifyCenter => crate::notifycenter::access(state),
        Surface::Balloons => Some(crate::notifyview::access(state)),
    }
}

/// Whether `s` takes the seat's keyboard from the focused window: the
/// menus, the Glass Start menu, the launcher and the overview, which take
/// every key while open and give the window back when they close without
/// acting. The clipboard history and the emoji picker type into the
/// window, so it keeps the keyboard; a dialog takes it on its own, the
/// taskbar too (`barkeys`), and the switcher ends on a window.
fn takes_keyboard(s: Surface) -> bool {
    matches!(
        s,
        Surface::Menus | Surface::StartPanel | Surface::Launcher | Surface::Overview
    )
}

/// Whether the seat's keyboard leaves the focused window: `focus` takes
/// it ([`takes_keyboard`]), or a balloon's reply field has the keys
/// (`reply`), so an input method serves the field instead of the window.
fn takes_keys(focus: Option<Surface>, reply: bool) -> bool {
    reply || focus.is_some_and(takes_keyboard)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HandOff {
    /// A popup that takes every key opened while a client's popup (a
    /// menu, a popover) holds the keyboard under its grab: close the
    /// client's popup, then take the keyboard from its window.
    Dismiss,
    /// A popup that takes every key opened over a window holding the
    /// keyboard: take it.
    Take,
    /// The popup is gone, no tree has the focus and nobody took the
    /// keyboard meanwhile: give it back to the window.
    GiveBack,
    /// The popup is gone but the keyboard went elsewhere (another
    /// window, a dialog, the taskbar, the lock): forget the loan.
    Forget,
    Keep,
}

/// [`HandOff`] for a pass where `popup` is whether a [`takes_keyboard`]
/// surface has the focus, `grabbed` whether a client popup's grab holds
/// the seat, `lent` whether one took the keyboard already, `held` whether
/// a client surface holds the seat's keyboard, `window` whether a window
/// is the active one, and `tree` whether any of the compositor's trees
/// has the focus.
fn hand_off(
    popup: bool,
    grabbed: bool,
    lent: bool,
    held: bool,
    window: bool,
    tree: bool,
) -> HandOff {
    match (popup, lent) {
        (true, _) if grabbed => HandOff::Dismiss,
        (true, false) if held && window => HandOff::Take,
        (false, true) if !held && window && !tree => HandOff::GiveBack,
        (false, true) if held || !window => HandOff::Forget,
        _ => HandOff::Keep,
    }
}

/// Move the seat's keyboard for open popups: a popup that takes every key ([`takes_keyboard`]) takes it from the window before its tree claims the focus and hands it back, as a fresh enter, when it lets go. A client's open popup closes first: its grab would keep the keyboard on it.
fn move_keyboard(state: &mut State, focus: Option<Surface>) {
    let held = state.seat.get_keyboard().and_then(|k| k.current_focus());
    let grabbed = state
        .seat
        .get_pointer()
        .is_some_and(|p| crate::popupgrab::holding(&p).is_some());
    let reply = crate::notifyview::reply_focused(state);
    let step = hand_off(
        takes_keys(focus, reply),
        grabbed,
        state.access.lent.is_some(),
        held.is_some(),
        state.focused.is_some(),
        focus.is_some() || reply,
    );
    match step {
        // The keyboard is back on the popup's parent: take it from there,
        // or take it again when ours already holds the loan.
        HandOff::Dismiss => {
            if !crate::popupgrab::dismiss(state) {
                return;
            }
            if state.access.lent.is_some() {
                state.enter_keyboard(None);
            } else {
                move_keyboard(state, focus);
            }
        }
        HandOff::Take => {
            state.enter_keyboard(None);
            let now = state.seat.get_keyboard().and_then(|k| k.current_focus());
            if now.is_none() {
                state.access.lent = held;
            }
        }
        HandOff::GiveBack => {
            let lent = state.access.lent.take();
            let Some(w) = state.focused else { return };
            let own = state.win(w).map(|x| x.wl_surface().clone());
            // The window's own surface, or one of its client's it held (a
            // popup of it) that is still there.
            let same = |s: &WlSurface| {
                s.is_alive() && own.as_ref().is_some_and(|o| o.client() == s.client())
            };
            match lent.filter(same) {
                Some(s) if !state.is_locked() => state.enter_keyboard(Some(s)),
                _ => state.claim_keyboard(w),
            }
        }
        HandOff::Forget => state.access.lent = None,
        HandOff::Keep => {}
    }
}

/// Match the adapters to the open surfaces, send each held tree what changed, then move the keyboard focus: away from the surface losing it before onto the one gaining it, so a reader hears them in order. The seat's keyboard moves around that ([`move_keyboard`]), so the window's adapter hears it lose and regain the focus around a popup.
fn sync(state: &mut State) {
    let focus = focus_of(&keyboard(state));
    let popup = takes_keys(focus, crate::notifyview::reply_focused(state));
    if popup {
        move_keyboard(state, focus);
    }
    if let Some(tx) = state.access.tx.clone() {
        publish(state, &tx, focus);
    }
    if !popup {
        move_keyboard(state, focus);
    }
}

fn publish(state: &mut State, tx: &Sender<Msg>, focus: Option<Surface>) {
    let live = surfaces(state);
    let mut slots = std::mem::take(&mut state.access.slots);
    slots.retain_mut(|slot| {
        let keep = live.contains(&slot.surface);
        if !keep && slot.focused {
            slot.adapter.update_window_focus_state(false);
        }
        keep
    });
    for &s in &live {
        if !slots.iter().any(|x| x.surface == s) {
            slots.push(Slot::new(s, tx));
        }
    }
    for slot in slots.iter_mut().filter(|x| x.active) {
        let (surface, published) = (slot.surface, &mut slot.published);
        slot.adapter.update_if_active(|| {
            let t = tree(state, surface).unwrap_or_else(|| Tree::window("", (0, 0)));
            published.update(t)
        });
        if slot.focused && focus != Some(surface) {
            slot.adapter.update_window_focus_state(false);
            slot.focused = false;
        }
    }
    if let Some(slot) = slots
        .iter_mut()
        .find(|x| x.active && !x.focused && Some(x.surface) == focus)
    {
        slot.adapter.update_window_focus_state(true);
        slot.focused = true;
    }
    state.access.slots = slots;
}

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

/// Whether `hit` lands on `surface`: a reader's click goes nowhere else,
/// not to a surface standing over the node it asked about.
fn lands_on(state: &State, surface: Surface, hit: &Hit) -> bool {
    match (surface, hit) {
        (Surface::Taskbar(t), Hit::Taskbar { bar, .. }) => *bar == t,
        (Surface::Dialog(id), Hit::Dialog { index, .. }) => {
            state.dialogs.views.get(*index).is_some_and(|v| v.id == id)
        }
        (Surface::Menus, Hit::Menu { .. })
        | (Surface::Switcher, Hit::Switcher { .. })
        | (Surface::Overview, Hit::Overview { .. })
        | (Surface::Launcher, Hit::Launcher { .. })
        | (Surface::StartPanel, Hit::StartPanel { .. })
        | (Surface::ClipView, Hit::ClipView { .. })
        | (Surface::EmojiView, Hit::EmojiView { .. })
        | (Surface::Calendar, Hit::Calendar { .. })
        | (Surface::Displays, Hit::Displays { .. })
        | (Surface::TrayFlyout, Hit::TrayFlyout { .. })
        | (Surface::TaskPreview, Hit::TaskPreview { .. })
        | (Surface::Balloons, Hit::Notification)
        | (Surface::NotifyCenter, Hit::NotifyCenter) => true,
        _ => false,
    }
}

/// Carry out a reader's `req` on a node of `surface`'s tree as last
/// published: the click `reprisekit::access::gesture` picks, at the
/// node's middle, if that point still lands on the surface. Focus on a
/// taskbar item gives the bar the keyboard there; on a menu row it
/// lights the row.
fn act(state: &mut State, surface: Surface, req: Request) {
    let target = (state.access.slots.iter())
        .find(|x| x.surface == surface && x.active)
        .and_then(|x| x.published.target(req.node));
    let Some(target) = target else {
        return;
    };
    let hit = state.scene_hit(Point::from(target.at));
    if !lands_on(state, surface, &hit) {
        log::debug!("access: {req:?} on {surface:?} lands on {hit:?}; dropped");
        return;
    }
    if let (Act::Focus, Hit::Taskbar { bar, hit }) = (req.act, &hit)
        && *hit != reprisekit::taskbar::TaskbarHit::Nothing
    {
        state.focus_bar_item(*bar, *hit);
        return;
    }
    if (req.act, &hit) == (Act::Focus, &Hit::NotifyCenter) {
        crate::notifycenter::select_at(state, target.at.0, target.at.1);
        return;
    }
    match reprisekit::access::gesture(req.act, target) {
        Some(Gesture::Click(at)) => click(state, at, BTN_LEFT),
        Some(Gesture::RightClick(at)) => click(state, at, BTN_RIGHT),
        Some(Gesture::Hover(_)) => {
            if let Hit::Menu { depth, row } = hit {
                state.menu_hover(depth, row);
            }
        }
        None => {}
    }
}

/// Click pointer `button` at layout point `at` as a press and release would, without moving the pointer: the press goes through `State::press`, the release through the interaction it started, with `State::pointer_location` reading `at` meanwhile. No client hears it. Nothing happens while locked, mid-gesture or with a button held.
pub fn click(state: &mut State, at: (f64, f64), button: u32) {
    if state.is_locked() || state.interaction != Interaction::Idle || state.buttons_down > 0 {
        return;
    }
    let real = state.pointer_location();
    let pos = Point::from(at);
    state.access.click_at = Some(pos);
    if state.switcher.is_open() {
        state.switcher_press(button);
        state.switcher_release(button);
    } else if state.overview.view.is_some() {
        state.overview_press(button);
        state.overview_release(button);
    } else {
        state.press(button, pos, InputTime::now());
        state.start_panel_release();
        if state.interaction != Interaction::Idle {
            state.end_interaction();
        }
    }
    state.access.click_at = None;
    state.repoint(real, InputTime::now());
    state.redraw_all();
}

/// Tell a listening screen reader about key `k` (xkb keycode `code`),
/// pressed or released, while one of the compositor's trees has the
/// keyboard. Never while locked: `focus_of` gives the lock screen no
/// tree. Call before the key updates the keyboard's state, so the text
/// is what the key types.
pub fn echo_key(state: &mut State, code: Keycode, k: &KeyPress, time: u32) {
    if !state.access.slots.iter().any(|x| x.active) || focus_of(&keyboard(state)).is_none() {
        return;
    }
    let Some(kb) = state.seat.get_keyboard() else {
        return;
    };
    let keysym = k.syms.first().copied().unwrap_or(0);
    let typed = kb.with_xkb_state(state, |ctx| {
        let x = ctx.xkb().lock().unwrap();
        // SAFETY: the state is only read; no reference outlives the lock.
        crate::editkeys::KeyText::of_key(unsafe { x.state() }, code)
    });
    let typed = typed.filter(|t| t.as_str().chars().all(|c| !c.is_control()));
    let text = match &typed {
        Some(t) => t.as_str().to_string(),
        None => crate::winmenu::keysym_name(keysym),
    };
    let note = KeyNote {
        pressed: k.pressed,
        keysym,
        keycode: code.raw(),
        mods: crate::keys::mask(&kb.modifier_state()),
        time,
        text: Zeroizing::new(text),
        is_text: typed.is_some(),
    };
    state
        .access
        .keys
        .get_or_insert_with(KeyEcho::spawn)
        .send(note);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keyboards_holder_has_the_focus_in_route_order() {
        let none = Keyboard::default();
        assert_eq!(focus_of(&none), None, "nothing of ours is open");
        let menu_over_dialog = Keyboard {
            menus: true,
            dialog: Some(3),
            ..Keyboard::default()
        };
        assert_eq!(focus_of(&menu_over_dialog), Some(Surface::Menus));
        let dialog = Keyboard {
            dialog: Some(3),
            client: true,
            ..Keyboard::default()
        };
        assert_eq!(focus_of(&dialog), Some(Surface::Dialog(3)));
        let locked = Keyboard {
            elsewhere: true,
            menus: true,
            ..Keyboard::default()
        };
        assert_eq!(focus_of(&locked), None, "the lock screen has no tree");
    }

    #[test]
    fn the_taskbar_with_the_keyboard_has_the_focus_under_a_menu_over_a_dialog() {
        let mut scene = crate::compositor::DeskScene::new();
        let t = scene.tree_create(scene.root());
        let bar = Keyboard {
            taskbar: Some(t),
            dialog: Some(3),
            client: true,
            ..Keyboard::default()
        };
        assert_eq!(focus_of(&bar), Some(Surface::Taskbar(t)));
        let menu = Keyboard { menus: true, ..bar };
        assert_eq!(
            focus_of(&menu),
            Some(Surface::Menus),
            "a menu the bar opened has it"
        );
    }

    #[test]
    fn only_popups_that_take_every_key_take_the_keyboard_from_the_window() {
        assert!(takes_keyboard(Surface::Menus));
        assert!(takes_keyboard(Surface::StartPanel));
        assert!(
            !takes_keyboard(Surface::EmojiView),
            "the emoji picker types into the window"
        );
        assert!(!takes_keyboard(Surface::ClipView));
        assert!(!takes_keyboard(Surface::Dialog(1)));
    }

    #[test]
    fn a_focused_reply_field_takes_the_keyboard_from_the_window() {
        assert!(takes_keys(None, true), "a reply field over a window");
        assert!(
            takes_keys(Some(Surface::ClipView), true),
            "the reply field has the keys under a panel that does not take them"
        );
        assert!(!takes_keys(None, false), "no reply field, no popup");
        assert!(!takes_keys(Some(Surface::ClipView), false));
        assert!(takes_keys(Some(Surface::Menus), false));
    }

    #[test]
    fn a_popup_takes_the_keyboard_from_the_window_and_gives_it_back() {
        assert_eq!(
            hand_off(true, false, false, true, true, true),
            HandOff::Take,
            "a menu opens over a window holding the keyboard"
        );
        assert_eq!(
            hand_off(true, false, false, false, true, true),
            HandOff::Keep,
            "nothing to take when no surface holds the keyboard"
        );
        assert_eq!(
            hand_off(true, false, false, true, false, true),
            HandOff::Keep,
            "a layer surface holding the keyboard keeps it"
        );
        assert_eq!(
            hand_off(true, false, true, false, true, true),
            HandOff::Keep,
            "a submenu keeps the loan"
        );
        assert_eq!(
            hand_off(false, false, true, false, true, false),
            HandOff::GiveBack,
            "the menu closed without acting"
        );
        assert_eq!(
            hand_off(false, false, true, false, true, true),
            HandOff::Keep,
            "a menu the taskbar opened closes back to the bar, which holds the loan"
        );
        assert_eq!(
            hand_off(false, false, true, true, true, false),
            HandOff::Forget,
            "a window took the keyboard meanwhile"
        );
        assert_eq!(
            hand_off(false, false, true, false, false, false),
            HandOff::Forget,
            "the window closed"
        );
        assert_eq!(
            hand_off(false, false, false, true, true, false),
            HandOff::Keep
        );
    }

    #[test]
    fn a_popup_that_takes_every_key_closes_a_clients_open_menu() {
        assert_eq!(
            hand_off(true, true, false, true, true, true),
            HandOff::Dismiss,
            "Start opens over an app's menu"
        );
        assert_eq!(
            hand_off(true, true, true, false, true, true),
            HandOff::Dismiss,
            "a client popup grabbed while a menu has the keyboard closes too"
        );
        assert_eq!(
            hand_off(true, true, false, true, false, true),
            HandOff::Dismiss,
            "a layer surface's popup closes as well"
        );
        assert_eq!(
            hand_off(false, true, false, true, true, false),
            HandOff::Keep,
            "an app's menu with none of ours open stays"
        );
    }

    #[test]
    fn a_focused_reply_field_has_the_focus_over_the_history() {
        let center = Keyboard {
            notify_center: true,
            client: true,
            ..Keyboard::default()
        };
        assert_eq!(focus_of(&center), Some(Surface::NotifyCenter));
        let reply = Keyboard {
            notify_reply: true,
            ..center
        };
        assert_eq!(focus_of(&reply), Some(Surface::Balloons));
        let emoji = Keyboard {
            emojiview: true,
            ..reply
        };
        assert_eq!(focus_of(&emoji), Some(Surface::EmojiView));
    }

    #[test]
    fn the_calendar_takes_the_focus_only_from_no_client() {
        let over_window = Keyboard {
            calendar: true,
            client: true,
            ..Keyboard::default()
        };
        assert_eq!(focus_of(&over_window), None);
        let alone = Keyboard {
            calendar: true,
            ..Keyboard::default()
        };
        assert_eq!(focus_of(&alone), Some(Surface::Calendar));
    }

    #[test]
    fn a_root_is_placed_by_a_transform_and_lays_out_from_zero() {
        let t = root(Role::Window, "Taskbar", Rect::new(100, 740, 1280, 30));
        let n = t.get(reprisekit::access::ROOT).expect("root");
        assert_eq!(n.label(), Some("Taskbar"));
        let b = n.bounds().expect("bounds");
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (0.0, 0.0, 1280.0, 30.0));
        assert_eq!(
            n.transform().map(|a| a.as_coeffs()),
            Some(Affine::translate((100.0, 740.0)).as_coeffs())
        );
    }
}
