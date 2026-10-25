//! System tray: StatusNotifierItem icons on the taskbar with DBusMenu popups. The D-Bus side lives on its own thread (`dbus.rs`).

pub mod builtin;
mod customize;
pub mod dbus;
mod icons;
mod menu;
pub mod notify;
mod place;
mod pointer;
pub mod screensaver;
pub mod settings;
#[cfg(test)]
mod tests;

pub(crate) use customize::customize_icon_landed;
pub use customize::{Remembered, apply_choices, customize};
pub use icons::{
    NOTIFY_SIZES, bar_icons, closest, decode_named_at, invalidate_icons, item_icon, item_pixmaps,
    sync_icon_sizes,
};
pub use menu::{
    DEFAULT_PROP, MenuNode, Submenus, forget_output, menu_activate, menu_hover, menus_closed,
    menus_closed_above,
};
pub use place::{behaviour_of, chevron_shown, mutes, overflow_enabled, placement, replace_all};
pub(crate) use pointer::bar_cell;
pub use pointer::{bell_cell, click, click_item, is_displays_cell, scroll_item, wheel_steps};

use icons::drawn_sizes;
use menu::{menu_open_for, open_layout};
use place::merge_item;

use crate::compositor::State;
use crate::raster::{Canvas, Rect};
use reprisekit::icon::IconSource;
use reprisescene::TreeId;
use smithay::output::Output;
use smithay::reexports::calloop::{
    Interest, Mode, PostAction, RegistrationToken, generic::Generic,
};
use std::collections::{BTreeMap, HashMap};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayItem {
    pub id: String,
    /// What its tray choice is kept under across restarts: its `Id`
    /// property, else the program serving it, else its bus name.
    pub key: String,
    pub service: String,
    pub path: String,
    pub title: String,
    /// What the taskbar tooltip shows: the `ToolTip` property's title, or
    /// `Title` when the item has none. Empty means no tip.
    pub tooltip: String,
    pub icon_name: Option<String>,
    pub icon_theme_path: Option<PathBuf>,
    pub pixmaps: Arc<[IconSource]>,
    pub named: Arc<[IconSource]>,
    pub menu_path: Option<String>,
    /// `Status` is `Passive`: the item asks not to be shown. It waits in
    /// `Tray::hidden`, shown only in the flyout behind the ^ cell when tray
    /// overflow is on.
    pub passive: bool,
    /// The process owning `service`, whose next window a click on the item
    /// lets take focus; `None` when the bus would not say.
    pub pid: Option<i32>,
}

impl TrayItem {
    /// What the item is spoken as: its tooltip, else its title.
    pub fn name(&self) -> String {
        if self.tooltip.is_empty() {
            self.title.clone()
        } else {
            self.tooltip.clone()
        }
    }
}

#[derive(Debug)]
pub enum TrayEvent {
    Added(TrayItem),
    Updated(TrayItem),
    Removed(String),
    MenuLayout {
        id: String,
        parent: i32,
        items: Vec<MenuNode>,
    },
    Unavailable(String),
    Notify(Notification),
    CloseRequest(u32),
    /// A notification's sender left the bus (its unique name): its actions
    /// have nobody to run them.
    NotifySenderGone(String),
    Customize,
    /// `org.reprise.Tray.CriticalTurnOff`: a critical battery turns the
    /// computer off without the session-end prompt.
    CriticalTurnOff,
    Identify(Vec<String>),
    PreviewScreenSaver(reprisekit::saver::Kind),
    /// Whether any `org.freedesktop.ScreenSaver` cookie is held: the idle
    /// timer is paused while one is.
    ScreenSaverInhibited(bool),
    /// `org.reprise.Tray.WindowIcon`, answered on `reply`.
    WindowIcon {
        lineages: Vec<Vec<crate::streamwindow::Step>>,
        title: String,
        reply: async_channel::Sender<reprisebus::tray::WindowIconWire>,
    },
}

/// The balloon's title and body limits: what a peer sends past them is
/// cut before it reaches the loop, so the wrap and the canvas stay
/// bounded.
pub const SUMMARY_CHARS: usize = 63;
pub const BODY_CHARS: usize = 255;

pub fn clip(mut s: String, max: usize) -> String {
    if let Some((i, _)) = s.char_indices().nth(max) {
        s.truncate(i);
    }
    s
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    pub id: u32,
    /// Who sent it: the `desktop-entry` hint's application name, else the
    /// `app_name` it gave. May be empty.
    pub app: String,
    pub summary: String,
    /// Plain text: any markup was reduced by `notify::plain_text`.
    pub body: String,
    /// The `app_icon` name, else the desktop entry's icon, decoded by the
    /// D-Bus thread; empty when neither resolves.
    pub app_icon: Vec<IconSource>,
    /// The `image-data` hint, else `image-path`, decoded by the D-Bus
    /// thread; empty when neither resolves.
    pub image: Vec<IconSource>,
    /// Milliseconds to stay up; -1 asks for the server default, 0 never
    /// expires on its own.
    pub expire_timeout: i32,
    /// (key, label) pairs in the sender's order, `default` among them when
    /// a click on the body should run something.
    pub actions: Vec<(String, String)>,
    pub urgency: notify::Urgency,
    /// Not kept in the history once its balloon goes.
    pub transient: bool,
    /// Invoking an action leaves it open.
    pub resident: bool,
    pub sender: String,
    pub pid: Option<i32>,
    pub reply: Option<Reply>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub placeholder: String,
    pub send: String,
}

pub const REPLY_SEND: &str = "Send";

impl Notification {
    pub fn new(id: u32, summary: String, body: String) -> Notification {
        Notification {
            id,
            app: String::new(),
            summary,
            body,
            app_icon: Vec::new(),
            image: Vec::new(),
            expire_timeout: -1,
            actions: Vec::new(),
            urgency: notify::Urgency::Normal,
            transient: false,
            resident: false,
            sender: String::new(),
            pid: None,
            reply: None,
        }
    }

    /// What a balloon draws as its one icon: the picture, else the app icon.
    pub fn balloon_icon(&self) -> &[IconSource] {
        if self.image.is_empty() {
            &self.app_icon
        } else {
            &self.image
        }
    }

    pub fn has_action(&self, key: &str) -> bool {
        self.actions.iter().any(|(k, _)| k == key)
    }

    pub fn button_actions(&self) -> impl Iterator<Item = &(String, String)> {
        self.actions
            .iter()
            .filter(|(k, _)| k != notify::DEFAULT_ACTION)
    }
}

#[derive(Debug)]
pub enum TrayCommand {
    /// A click on the icon, reported at layout (`x`, `y`): over the
    /// icon's centre, on the bar's or flyout's edge facing the screen.
    /// `token` goes to the item through `ProvideXdgActivationToken` first.
    Click {
        id: String,
        x: i32,
        y: i32,
        kind: ClickKind,
        token: Option<String>,
    },
    /// A wheel step over the icon. `delta` is in wheel steps in the item's
    /// own convention, positive away from the user (up, or left).
    Scroll {
        id: String,
        delta: i32,
        orientation: Orientation,
    },
    GetLayout {
        id: String,
        parent: i32,
    },
    /// A menu row picked; `token` goes to the item through
    /// `ProvideXdgActivationToken` first, as for `Click`.
    MenuEvent {
        id: String,
        item: i32,
        token: String,
    },
    /// Tell the bus a balloon closed, so `NotificationClosed` fires.
    /// `reason` is the freedesktop code (1 expired, 2 dismissed, 3 by call).
    NotificationClosed {
        id: u32,
        reason: u32,
    },
    Replied {
        id: u32,
        text: String,
    },
    ActionInvoked {
        id: u32,
        key: String,
        token: Option<String>,
    },
    Appearance(settings::Appearance),
    IconTheme(String),
    /// The px item icons are drawn at now ([`icons::icon_sizes`]): every item is read again and its
    /// icon decoded at them.
    IconSizes(Vec<i32>),
    Shutdown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClickKind {
    Activate,
    SecondaryActivate,
    ContextMenu,
}

impl ClickKind {
    pub fn method(self) -> &'static str {
        match self {
            ClickKind::Activate => "Activate",
            ClickKind::SecondaryActivate => "SecondaryActivate",
            ClickKind::ContextMenu => "ContextMenu",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    Vertical,
    Horizontal,
}

impl Orientation {
    pub fn as_str(self) -> &'static str {
        match self {
            Orientation::Vertical => "vertical",
            Orientation::Horizontal => "horizontal",
        }
    }
}

pub struct Tray {
    pub items: Vec<TrayItem>,
    /// The items behind the ^ cell, in the order they went there: Passive
    /// ones left to their status, and those set to hide. Empty while
    /// overflow is off.
    pub hidden: Vec<TrayItem>,
    pub choices: BTreeMap<String, Remembered>,
    pub flyout: Option<crate::trayflyout::FlyoutView>,
    pub events: Receiver<TrayEvent>,
    pub commands: async_channel::Sender<TrayCommand>,
    source: RegistrationToken,
    thread: Option<JoinHandle<()>>,
    /// A root menu requested by a right click, waiting for its layout:
    /// (item id, output, the icon's cell in layout coordinates, the click
    /// point the item hears).
    pub pending_menu: Option<(String, Output, Rect, (i32, i32))>,
    pub submenus: Submenus,
    /// Set once the D-Bus thread is known to be gone, so the warning is
    /// logged once instead of on every command.
    pub gone: bool,
    /// Wheel motion over a tray icon short of a whole notch: the cell index,
    /// whether the axis was horizontal, and the leftover 120ths (hi-res
    /// wheels send fractions of a notch).
    wheel: Option<(usize, bool, i32)>,
    /// `item_icon`'s answers by (item id, px), dropped whenever an item
    /// changes: a redraw must not rescale every pixmap.
    icon_cache: HashMap<(String, i32), Option<Arc<Canvas>>>,
    /// The [`icons::icon_sizes`] the D-Bus thread decodes item icons at.
    icon_sizes: Vec<i32>,
}

impl Tray {
    pub fn shown(&self) -> &[TrayItem] {
        &self.items[..self.items.len().min(MAX_ITEMS)]
    }

    /// Queue a command for the D-Bus thread. Once that thread is gone every
    /// further command is dropped without a word; the first one says so.
    fn send(&mut self, cmd: TrayCommand) {
        if self.gone {
            return;
        }
        if self.commands.try_send(cmd).is_err() {
            log::warn!("tray: D-Bus thread is gone; dropping tray commands");
            self.gone = true;
        }
    }

    /// Every item, those on the bar first.
    fn all(&self) -> impl Iterator<Item = &TrayItem> {
        self.items.iter().chain(&self.hidden)
    }

    pub fn item(&self, id: &str) -> Option<&TrayItem> {
        self.all().find(|i| i.id == id)
    }

    pub fn item_at(&self, revealed: usize, leading: usize, index: usize) -> Option<&TrayItem> {
        match Cell::at(revealed, self.items.len(), leading, index) {
            Cell::Item(i) => self.items.get(i),
            Cell::Hidden(i) => self
                .hidden
                .get(hidden_index(self.hidden.len(), revealed, i)?),
            Cell::Builtin(_) => None,
        }
    }
}

/// How many tray items the taskbar draws. A peer can register any number of
/// items; past this the cells would run into the task buttons.
pub const MAX_ITEMS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    Hidden(usize),
    Item(usize),
    Builtin(usize),
}

impl Cell {
    pub fn at(revealed: usize, items: usize, leading: usize, index: usize) -> Cell {
        if index < revealed {
            return Cell::Hidden(index);
        }
        let index = index - revealed;
        let shown = items.min(MAX_ITEMS);
        if index < leading {
            Cell::Builtin(index)
        } else if index - leading < shown {
            Cell::Item(index - leading)
        } else {
            Cell::Builtin(index - shown)
        }
    }
}

/// Position in `Tray::hidden` (of `len` items) of revealed cell `i` when
/// `revealed` are open: the run's last items, nearest the shown icons.
/// `None` when the run shrank under the draw.
fn hidden_index(len: usize, revealed: usize, i: usize) -> Option<usize> {
    (len + i)
        .checked_sub(revealed)
        .filter(|&k| k < len && i < revealed)
}

pub fn keep_run<T>(icons: &mut Vec<T>, run: usize, kept: usize) {
    icons.drain(..run.saturating_sub(kept));
}

pub fn leading(state: &State) -> usize {
    builtin::shown(state).iter().filter(|b| b.leads()).count()
}

/// The process owning the StatusNotifier item in bar `bar`'s tray cell `index`.
pub fn item_pid(state: &State, bar: TreeId, index: usize) -> Option<i32> {
    let tray = state.tray.as_ref()?;
    tray.item_at(revealed(state, bar), leading(state), index)?
        .pid
}

/// How many hidden items bar `bar` shows in front of its icons.
fn revealed(state: &State, bar: TreeId) -> usize {
    state.bar(bar).map_or(0, |b| b.layout.tray_hidden)
}

pub fn cell_at(state: &State, bar: TreeId, index: usize) -> Cell {
    let revealed = revealed(state, bar);
    let items = state.tray.as_ref().map_or(0, |t| t.items.len());
    Cell::at(revealed, items, leading(state), index)
}

fn tray_readable(state: &mut State, fd: BorrowedFd) {
    let mut counter = 0u64;
    // SAFETY: `fd` is the open eventfd and `counter` a live 8-byte local.
    // The result is ignored: EAGAIN only means another wake drained it.
    unsafe {
        libc::read(
            fd.as_raw_fd(),
            &mut counter as *mut u64 as *mut libc::c_void,
            8,
        );
    }
    // Take the events out first: handling one may reach the tray again.
    let mut pending = Vec::new();
    if let Some(tray) = state.tray.as_ref() {
        while let Ok(ev) = tray.events.try_recv() {
            pending.push(ev);
        }
    }
    let mut redraw = false;
    for ev in pending {
        redraw |= handle_event(state, ev);
    }
    if redraw {
        state.redraw_taskbars();
    }
}

fn new_eventfd() -> Option<OwnedFd> {
    // SAFETY: plain syscall, no pointers.
    let raw = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    // SAFETY: `raw` is a fresh fd nothing else owns.
    (raw >= 0).then(|| unsafe { OwnedFd::from_raw_fd(raw) })
}

pub fn setup(state: &mut State) {
    let Some(eventfd) = new_eventfd() else {
        log::warn!("tray: eventfd failed; tray disabled");
        return;
    };
    // The thread gets a dup of its own: each side closes its own number.
    let Ok(thread_fd) = eventfd.try_clone() else {
        log::warn!("tray: dup failed; tray disabled");
        return;
    };
    let (tx, rx) = std::sync::mpsc::channel::<TrayEvent>();
    let appearance = settings::Appearance::live(state);
    let icon_theme = state.config.icons.theme.clone();
    let pads = state.devices.pads.shared();
    let sizes = drawn_sizes(state);
    let Some((commands, thread)) = dbus::start(tx, thread_fd, appearance, icon_theme, pads, sizes)
    else {
        log::warn!("tray: could not start the D-Bus thread; tray disabled");
        return;
    };
    install(state, eventfd, rx, commands, Some(thread));
}

/// The tray of a test, with no D-Bus thread: what is sent on the returned channel arrives
/// once the returned eventfd is written, as the thread's events do. The receiver hears the
/// commands the thread would.
#[cfg(test)]
pub fn headless(
    state: &mut State,
) -> (
    std::sync::mpsc::Sender<TrayEvent>,
    OwnedFd,
    async_channel::Receiver<TrayCommand>,
) {
    let eventfd = new_eventfd().expect("eventfd");
    let wake = eventfd.try_clone().expect("dup");
    let (tx, rx) = std::sync::mpsc::channel::<TrayEvent>();
    let (commands, heard) = async_channel::unbounded();
    install(state, eventfd, rx, commands, None);
    (tx, wake, heard)
}

/// The tray reading `events` whenever `eventfd` is written.
fn install(
    state: &mut State,
    eventfd: OwnedFd,
    events: std::sync::mpsc::Receiver<TrayEvent>,
    commands: async_channel::Sender<TrayCommand>,
    thread: Option<JoinHandle<()>>,
) {
    let source = Generic::new(eventfd, Interest::READ, Mode::Level);
    let token = state.handle.insert_source(source, |_, fd, state| {
        tray_readable(state, fd.as_fd());
        Ok(PostAction::Continue)
    });
    let source = match token {
        Ok(t) => t,
        Err(e) => {
            // Nothing would drain the event queue, so the thread must not
            // fill it.
            log::warn!(
                "tray: could not watch the eventfd ({}); tray disabled",
                e.error
            );
            let _ = commands.try_send(TrayCommand::Shutdown);
            return;
        }
    };
    state.tray = Some(Tray {
        items: Vec::new(),
        hidden: Vec::new(),
        choices: crate::state::load().tray_icons,
        flyout: None,
        events,
        commands,
        source,
        thread,
        pending_menu: None,
        submenus: Submenus::default(),
        gone: false,
        wheel: None,
        icon_cache: HashMap::new(),
        icon_sizes: drawn_sizes(state),
    });
}

pub fn shutdown(state: &mut State) {
    crate::trayflyout::close(state);
    let Some(mut tray) = state.tray.take() else {
        return;
    };
    let _ = tray.commands.try_send(TrayCommand::Shutdown);
    state.handle.remove(tray.source);
    // No join: the thread has its own eventfd dup and ends after Shutdown
    // or with the process.
    drop(tray.thread.take());
}

pub fn notification_closed(state: &mut State, id: u32, reason: u32) {
    send(state, TrayCommand::NotificationClosed { id, reason });
}

/// Queue a command for the D-Bus thread, if the tray is up. A click or a
/// menu row is the user acting on the item, so the window it opens takes
/// focus (`mapfocus::grant`).
pub(crate) fn send(state: &mut State, cmd: TrayCommand) {
    let Some(tray) = state.tray.as_mut() else {
        return;
    };
    if let TrayCommand::Click { id, .. } | TrayCommand::MenuEvent { id, .. } = &cmd
        && let Some(pid) = tray.item(id).and_then(|i| i.pid)
    {
        crate::mapfocus::grant(pid);
    }
    tray.send(cmd);
}

fn handle_event(state: &mut State, ev: TrayEvent) -> bool {
    match ev {
        TrayEvent::Added(item) | TrayEvent::Updated(item) => {
            let overflow = overflow_enabled(&state.config);
            let Some(tray) = state.tray.as_mut() else {
                return false;
            };
            let id = item.id.clone();
            let place = placement(&item, behaviour_of(&tray.choices, &item), overflow);
            let redraw = merge_item(&mut tray.items, &mut tray.hidden, item, place);
            if redraw {
                tray.icon_cache.retain(|(cached, _), _| *cached != id);
            } else {
                crate::access::touch(state);
            }
            redraw
        }
        TrayEvent::Removed(id) => {
            let Some(tray) = state.tray.as_mut() else {
                return false;
            };
            tray.icon_cache.retain(|(cached, _), _| *cached != id);
            tray.items.retain(|i| i.id != id);
            tray.hidden.retain(|i| i.id != id);
            if tray
                .pending_menu
                .as_ref()
                .is_some_and(|(pid, ..)| *pid == id)
            {
                tray.pending_menu = None;
            }
            tray.submenus.forget_item(&id);
            if menu_open_for(state, &id) {
                state.close_menu();
            }
            true
        }
        TrayEvent::MenuLayout { id, parent, items } => {
            open_layout(state, id, parent, items);
            false
        }
        TrayEvent::Unavailable(reason) => {
            if let Some(tray) = state.tray.as_mut()
                && !tray.gone
            {
                log::warn!("tray unavailable: {reason}");
                // The thread stops after this, so commands go nowhere.
                tray.gone = true;
            }
            false
        }
        TrayEvent::Notify(n) => {
            if mutes(state, n.pid) {
                notification_closed(state, n.id, crate::notifyview::REASON_DISMISSED);
            } else {
                crate::notifyview::show(state, n);
            }
            false
        }
        TrayEvent::Customize => {
            customize(state);
            false
        }
        TrayEvent::CriticalTurnOff => {
            let cmd = state.config.power.poweroff.clone();
            crate::sessionend::begin_critical(state, cmd);
            false
        }
        TrayEvent::Identify(names) => {
            crate::identifyview::show(state, &names);
            false
        }
        TrayEvent::PreviewScreenSaver(kind) => {
            crate::saverview::start(state, kind, true);
            false
        }
        TrayEvent::CloseRequest(id) => {
            crate::notifyview::close(state, id, crate::notifyview::REASON_CLOSED);
            false
        }
        TrayEvent::NotifySenderGone(name) => {
            crate::notifyview::sender_gone(state, &name);
            false
        }
        TrayEvent::ScreenSaverInhibited(on) => {
            log::debug!("tray: screensaver inhibited: {on}");
            state.idle.bus_inhibited = on;
            crate::idle::inhibition_changed(state);
            false
        }
        TrayEvent::WindowIcon {
            lineages,
            title,
            reply,
        } => {
            let found = crate::streamwindow::window_icon(state, &lineages, &title);
            let _ = reply.try_send(reprisebus::tray::WindowIcon::wire(found));
            false
        }
    }
}
