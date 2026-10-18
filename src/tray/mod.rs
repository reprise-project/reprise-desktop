//! System tray: StatusNotifierItem icons on the taskbar with DBusMenu popups. The D-Bus side lives on its own thread (`dbus.rs`).

pub mod builtin;
pub mod dbus;
pub mod notify;
pub mod screensaver;
pub mod settings;

use crate::compositor::State;
use crate::icontheme::IconCache;
use crate::input::{BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, Hit};
use crate::menuview::{MenuFrom, MenuKind};
use crate::raster::{Canvas, Rect};
use crate::startmenu::{MenuItem, MenuLayout, MenuStyle, place_submenu, submenu_max_rows};
use crate::taskbar::TaskbarHit;
use reprisekit::controls::traycustomize::{self, Behaviour, Choices, Entry, TrayCustomize};
use reprisekit::icon::{IconSource, premultiply};
use reprisescene::TreeId;
use serde::{Deserialize, Serialize};
use smithay::input::pointer::AxisFrame;
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuNode {
    pub id: i32,
    pub label: String,
    pub separator: bool,
    pub enabled: bool,
    pub visible: bool,
    pub checked: Option<bool>,
    pub radio: bool,
    pub has_children: bool,
    pub default: bool,
}

pub const DEFAULT_PROP: &str = "x-reprise-default";

/// Remove DBusMenu mnemonic markers: a single `_` is dropped, `__` becomes `_`.
pub fn strip_mnemonic(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                chars.next();
                out.push('_');
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Largest `IconPixmap` side the tray keeps. It draws 16 to 32 px, so a
/// bigger entry only costs memory.
const MAX_PIXMAP_SIDE: i32 = 256;

const PIXMAP_SIZES: [i32; 4] = [16, 22, 32, 48];

pub fn item_pixmaps(raw: &[(i32, i32, &[u8])]) -> Vec<IconSource> {
    let usable = || {
        raw.iter()
            .enumerate()
            .filter(|(_, (w, h, b))| pixmap_len(*w, *h, b.len()).is_some())
    };
    let mut picked: Vec<usize> = Vec::with_capacity(PIXMAP_SIZES.len());
    for size in PIXMAP_SIZES {
        if let Some((i, _)) = usable().min_by_key(|(_, (w, ..))| (w - size).abs())
            && !picked.contains(&i)
        {
            picked.push(i);
        }
    }
    if picked.is_empty() && !raw.is_empty() {
        log::warn!("tray: no usable IconPixmap among {} entries", raw.len());
    }
    picked
        .into_iter()
        .filter_map(|i| pixmap_to_source(raw[i].0, raw[i].1, raw[i].2))
        .collect()
}

/// Byte length of a `w`x`h` ARGB32 pixmap, or `None` for a non-positive
/// side, a side past [`MAX_PIXMAP_SIDE`] (an SNI peer controls `w`/`h`), or
/// a buffer of `have` bytes too short for it.
fn pixmap_len(w: i32, h: i32, have: usize) -> Option<usize> {
    if !(1..=MAX_PIXMAP_SIDE).contains(&w) || !(1..=MAX_PIXMAP_SIDE).contains(&h) {
        return None;
    }
    let len = w as usize * h as usize * 4;
    (have >= len).then_some(len)
}

/// Convert one `IconPixmap` entry (ARGB32, big-endian per pixel, straight
/// alpha) into premultiplied source pixels; `None` where [`pixmap_len`]
/// rejects it.
fn pixmap_to_source(w: i32, h: i32, argb_be: &[u8]) -> Option<IconSource> {
    let len = pixmap_len(w, h, argb_be.len())?;
    let pixels = argb_be[..len]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| premultiply(u32::from_be_bytes(*p)))
        .collect();
    Some(IconSource {
        width: w as u32,
        height: h as u32,
        pixels,
    })
}

/// Sizes a peer-named icon is decoded at. The tray and the balloon draw
/// at 16 px per step of output scale; past 2x the 32 px copy is scaled.
const NAMED_SIZES: [i32; 2] = [16, 32];

/// Sizes a notification's icons are decoded at; 48 and 96 px feed the Glass sheet's icon slot.
pub const NOTIFY_SIZES: [i32; 4] = [16, 32, 48, 96];

/// A peer-named icon (a theme name or an absolute path, with `extra`
/// directories searched first) decoded at each of [`NAMED_SIZES`]. Only
/// the D-Bus thread calls this: the file is the peer's choice, and even an
/// SVG inside the decoder's budget can take a second to draw.
pub fn decode_named(icons: &mut IconCache, name: &str, extra: &[PathBuf]) -> Vec<IconSource> {
    decode_named_at(icons, name, extra, &NAMED_SIZES)
}

/// [`decode_named`] at the given `sizes`.
pub fn decode_named_at(
    icons: &mut IconCache,
    name: &str,
    extra: &[PathBuf],
    sizes: &[i32],
) -> Vec<IconSource> {
    sizes
        .iter()
        .filter_map(|&px| icons.get_with_extra(name, px, extra))
        .map(|c| IconSource {
            width: c.width,
            height: c.height,
            pixels: c.pixels,
        })
        .collect()
}

pub fn closest(sources: &[IconSource], px: i32) -> Option<Canvas> {
    sources
        .iter()
        .min_by_key(|s| (s.width as i32 - px).abs())
        .map(|s| s.to_canvas(px))
}

/// Map DBusMenu nodes to menu items, skipping invisible ones. The second
/// vector runs parallel to the first and carries the node id to send back
/// on click (`None` for separators).
pub fn menu_items(nodes: &[MenuNode]) -> (Vec<MenuItem>, Vec<Option<i32>>) {
    let mut items = Vec::with_capacity(nodes.len());
    let mut ids = Vec::with_capacity(nodes.len());
    for n in nodes.iter().filter(|n| n.visible) {
        if n.separator {
            items.push(MenuItem::Separator);
            ids.push(None);
        } else {
            items.push(
                MenuItem::entry(strip_mnemonic(&n.label))
                    .enabled(n.enabled)
                    .checked(n.checked.unwrap_or(false))
                    .radio(n.radio)
                    .submenu(n.has_children)
                    .bold(n.default),
            );
            ids.push(Some(n.id));
        }
    }
    (items, ids)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remembered {
    pub behaviour: Behaviour,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
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

    pub fn item(&self, id: &str) -> Option<&TrayItem> {
        self.items.iter().chain(&self.hidden).find(|i| i.id == id)
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmenuRequest {
    pub id: String,
    pub node: i32,
    pub depth: usize,
    pub row: usize,
}

/// Tray submenu bookkeeping: the request in flight, and the parent row every
/// open submenu hangs off (`open_rows[d]` is the row of the menu at depth `d`
/// that opened the menu at `d + 1`). Pure because the cases are races: a reply
/// landing after the menu was dismissed or the pointer moved on.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Submenus {
    pub pending: Option<SubmenuRequest>,
    open_rows: Vec<usize>,
}

impl Submenus {
    pub fn needs_request(&self, id: &str, depth: usize, row: usize) -> bool {
        if self.open_rows.get(depth) == Some(&row) {
            return false;
        }
        !matches!(&self.pending, Some(p) if p.id == id && p.depth == depth && p.row == row)
    }

    pub fn request(&mut self, req: SubmenuRequest) {
        self.pending = Some(req);
    }

    /// Match a `MenuLayout` reply against the request in flight and consume
    /// it: `Some((depth, row))` for the reply we asked for. A reply for
    /// another item, another node (the pointer moved on) or a menu that has
    /// since closed is dropped.
    pub fn accept(&mut self, id: &str, parent: i32) -> Option<(usize, usize)> {
        let p = self.pending.as_ref()?;
        if p.id != id || p.node != parent {
            return None;
        }
        let at = (p.depth, p.row);
        self.pending = None;
        Some(at)
    }

    pub fn opened(&mut self, depth: usize, row: usize) {
        self.open_rows.truncate(depth);
        // Never reached in practice (submenus open depth by depth); the
        // filler cannot collide with a real row.
        self.open_rows.resize(depth, usize::MAX);
        self.open_rows.push(row);
    }

    pub fn closed_above(&mut self, depth: usize) {
        self.open_rows.truncate(depth);
        if self.pending.as_ref().is_some_and(|p| p.depth >= depth) {
            self.pending = None;
        }
    }

    pub fn clear(&mut self) {
        self.pending = None;
        self.open_rows.clear();
    }

    pub fn forget_item(&mut self, id: &str) {
        if self.pending.as_ref().is_some_and(|p| p.id == id) {
            self.pending = None;
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

pub fn cell_at(state: &State, bar: TreeId, index: usize) -> Cell {
    let revealed = state.bar(bar).map_or(0, |b| b.layout.tray_hidden);
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

pub fn setup(state: &mut State) {
    // SAFETY: plain syscall, no pointers.
    let raw = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    if raw < 0 {
        log::warn!("tray: eventfd failed; tray disabled");
        return;
    }
    // SAFETY: `raw` is a fresh fd nothing else owns.
    let eventfd = unsafe { OwnedFd::from_raw_fd(raw) };
    // The thread gets a dup of its own: each side closes its own number.
    let Ok(thread_fd) = eventfd.try_clone() else {
        log::warn!("tray: dup failed; tray disabled");
        return;
    };
    let (tx, rx) = std::sync::mpsc::channel::<TrayEvent>();
    let appearance = settings::Appearance::live(state);
    let icon_theme = state.config.icons.theme.clone();
    let pads = state.devices.pads.shared();
    let Some((commands, thread)) = dbus::start(tx, thread_fd, appearance, icon_theme, pads) else {
        log::warn!("tray: could not start the D-Bus thread; tray disabled");
        return;
    };
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
        events: rx,
        commands,
        source,
        thread: Some(thread),
        pending_menu: None,
        submenus: Submenus::default(),
        gone: false,
        wheel: None,
        icon_cache: HashMap::new(),
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

/// Tray overflow is on: `[taskbar] tray_overflow`, on by default under
/// Glass. Off, Passive items stay off the bar and nothing shows them.
pub fn overflow_enabled(config: &crate::config::Config) -> bool {
    config.taskbar.tray_overflow
}

pub fn chevron_shown(state: &State) -> bool {
    shows_chevron(
        &state.config,
        state.tray.as_ref().map_or(0, |t| t.hidden.len()),
    )
}

fn shows_chevron(config: &crate::config::Config, hidden: usize) -> bool {
    overflow_enabled(config) && hidden > 0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Bar,
    Hidden,
}

pub fn behaviour_of(choices: &BTreeMap<String, Remembered>, item: &TrayItem) -> Behaviour {
    choices
        .get(&item.key)
        .map_or(Behaviour::Auto, |r| r.behaviour)
}

/// Where `item` goes with `behaviour`: with overflow on, behind the ^ cell
/// when the behaviour hides it (its status decides when left Auto); with
/// overflow off every item is on the bar.
pub fn placement(item: &TrayItem, behaviour: Behaviour, overflow: bool) -> Place {
    if overflow && behaviour.hidden(item.passive) {
        Place::Hidden
    } else {
        Place::Bar
    }
}

/// Whether `a` and `b` look the same in a tray cell: the same icon from
/// the same sources and the same status. The tooltip, title, menu and pid
/// are read when used, never drawn.
fn draws_alike(a: &TrayItem, b: &TrayItem) -> bool {
    a.pixmaps == b.pixmaps
        && a.named == b.named
        && a.icon_name == b.icon_name
        && a.icon_theme_path == b.icon_theme_path
        && a.passive == b.passive
}

/// Put a fresh copy of an item at `place`, taking it out of the other
/// list when it moved. A known item keeps its place in its list; a new
/// one goes last. Returns whether the tray draws differently: a new item,
/// or one whose cell changed (`draws_alike`).
fn merge_item(
    items: &mut Vec<TrayItem>,
    hidden: &mut Vec<TrayItem>,
    item: TrayItem,
    place: Place,
) -> bool {
    let changed = items
        .iter()
        .chain(hidden.iter())
        .find(|i| i.id == item.id)
        .is_none_or(|old| !draws_alike(old, &item));
    let (to, from) = match place {
        Place::Bar => (items, hidden),
        Place::Hidden => (hidden, items),
    };
    from.retain(|i| i.id != item.id);
    match to.iter().position(|i| i.id == item.id) {
        Some(i) => to[i] = item,
        None => to.push(item),
    }
    changed
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
            let open_for_it = matches!(
                state.menus.first().map(|m| &m.kind),
                Some(MenuKind::Tray { id: open, .. }) if *open == id
            );
            if open_for_it {
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
    }
}

/// Put every item back where its behaviour and tray overflow put it,
/// after the choices or `[taskbar] tray_overflow` changed. Items keep
/// their order within a list; one that moves goes last in the other.
pub fn replace_all(state: &mut State) {
    let overflow = overflow_enabled(&state.config);
    let Some(tray) = state.tray.as_mut() else {
        return;
    };
    arrange(&mut tray.items, &mut tray.hidden, &tray.choices, overflow);
    state.redraw_taskbars();
}

fn arrange(
    items: &mut Vec<TrayItem>,
    hidden: &mut Vec<TrayItem>,
    choices: &BTreeMap<String, Remembered>,
    overflow: bool,
) {
    let at = |i: &TrayItem| placement(i, behaviour_of(choices, i), overflow);
    let (stay, to_hide): (Vec<_>, Vec<_>) = items.drain(..).partition(|i| at(i) == Place::Bar);
    let (to_bar, keep): (Vec<_>, Vec<_>) = hidden.drain(..).partition(|i| at(i) == Place::Bar);
    items.extend(stay.into_iter().chain(to_bar));
    hidden.extend(keep.into_iter().chain(to_hide));
}

/// Whether a balloon from process `pid` is held back: tray overflow is on
/// and an item that process serves hides its icon and its notifications.
pub fn mutes(state: &State, pid: Option<i32>) -> bool {
    let (Some(pid), Some(tray)) = (pid, state.tray.as_ref()) else {
        return false;
    };
    overflow_enabled(&state.config)
        && tray
            .items
            .iter()
            .chain(&tray.hidden)
            .any(|i| i.pid == Some(pid) && !behaviour_of(&tray.choices, i).balloons())
}

fn item_name(item: &TrayItem) -> &str {
    first_line(&[&item.tooltip, &item.title, &item.key])
}

fn first_line<'a>(texts: &[&'a String]) -> &'a str {
    texts
        .iter()
        .find_map(|s| s.lines().map(str::trim).find(|l| !l.is_empty()))
        .unwrap_or("")
}

pub fn customize(state: &mut State) {
    crate::trayflyout::close(state);
    let Some(tray) = state.tray.as_ref() else {
        return;
    };
    let theme = &state.config.theme;
    let wording = traycustomize::Wording::of(theme);
    let mut keys = std::collections::HashSet::new();
    let current: Vec<Entry> = tray
        .items
        .iter()
        .chain(&tray.hidden)
        .map(|i| {
            keys.insert(i.key.clone());
            // Glass names the item by its title and puts its tip under
            // that; Classic lists it by its tip.
            let (name, detail) = match wording {
                traycustomize::Wording::Glass => (
                    first_line(&[&i.title, &i.key]).to_string(),
                    first_line(&[&i.tooltip]).to_string(),
                ),
                traycustomize::Wording::Classic => (item_name(i).to_string(), String::new()),
            };
            Entry {
                key: i.key.clone(),
                name,
                detail,
                icon: if i.pixmaps.is_empty() {
                    i.named.clone()
                } else {
                    i.pixmaps.clone()
                },
                passive: i.passive,
                behaviour: behaviour_of(&tray.choices, i),
            }
        })
        .collect();
    let past_keys: Vec<(String, Remembered)> = tray
        .choices
        .iter()
        .filter(|(k, _)| !keys.contains(*k))
        .map(|(k, r)| (k.clone(), r.clone()))
        .collect();
    let all_icons =
        (wording == traycustomize::Wording::Glass).then(|| !overflow_enabled(&state.config));
    let metrics = theme.look.controls();
    let past = past_keys
        .into_iter()
        .map(|(key, r)| {
            let icon = past_icon(&mut state.icons, r.icon.as_deref());
            let name = if r.name.is_empty() {
                key.clone()
            } else {
                r.name
            };
            let detail = match wording {
                traycustomize::Wording::Glass => name.clone(),
                traycustomize::Wording::Classic => String::new(),
            };
            Entry {
                name,
                detail,
                key,
                icon: icon.into(),
                passive: false,
                behaviour: r.behaviour,
            }
        })
        .collect();
    let t = TrayCustomize::new(wording, current, past, all_icons, metrics);
    state.open_dialog(crate::dialog::Dialog::tray_customize(t));
}

/// `choices` after the Customize dialog's OK picked `picked`: an Auto
/// pick forgets its key, any other keeps it with the name and icon of the
/// registered item that has it, or what was kept before.
fn remember(
    choices: &mut BTreeMap<String, Remembered>,
    picked: &[(String, Behaviour)],
    current: &[&TrayItem],
) {
    for (key, b) in picked {
        if *b == Behaviour::Auto {
            choices.remove(key);
            continue;
        }
        let kept = choices.remove(key);
        let (name, icon) = match current.iter().find(|i| i.key == *key) {
            Some(i) => (item_name(i).to_string(), i.icon_name.clone()),
            None => kept.map_or_else(|| (key.clone(), None), |r| (r.name, r.icon)),
        };
        choices.insert(
            key.clone(),
            Remembered {
                behaviour: *b,
                name,
                icon,
            },
        );
    }
}

pub fn apply_choices(state: &mut State, picked: Choices) {
    if let Some(all) = picked.all_icons
        && all == state.config.taskbar.tray_overflow
    {
        state.config.taskbar.tray_overflow = !all;
        let key = repriseconf::key("taskbar", "tray_overflow");
        crate::configwrite::save_flag(&state.config_path, key, !all);
    }
    let Some(tray) = state.tray.as_mut() else {
        return;
    };
    let current: Vec<&TrayItem> = tray.items.iter().chain(&tray.hidden).collect();
    let mut choices = tray.choices.clone();
    remember(&mut choices, &picked.behaviours, &current);
    if choices != tray.choices {
        crate::state::save_tray_icons(&choices);
        tray.choices = choices;
    }
    replace_all(state);
}

/// A remembered item's icon for the Customize list. A file is never read on the loop: one not
/// decoded yet shows the stand-in, and the next Customize shows the file.
fn past_icon(icons: &mut IconCache, name: Option<&str>) -> Vec<IconSource> {
    let name = name.unwrap_or(crate::icon::STAND_IN);
    [16, 32]
        .into_iter()
        .filter_map(|px| crate::icon::unread(icons, name, px))
        .map(|c| IconSource {
            width: c.width,
            height: c.height,
            pixels: c.pixels.clone(),
        })
        .collect()
}

pub fn item_icon(state: &mut State, item: &TrayItem, px: i32) -> Option<Arc<Canvas>> {
    let key = (item.id.clone(), px);
    if let Some(c) = state.tray.as_ref().and_then(|t| t.icon_cache.get(&key)) {
        return c.clone();
    }
    let found = closest(&item.pixmaps, px)
        .or_else(|| closest(&item.named, px))
        .or_else(|| state.icons.get("application-x-executable", px))
        .map(Arc::new);
    if let Some(t) = state.tray.as_mut() {
        t.icon_cache.insert(key, found.clone());
    }
    found
}

pub fn bar_icons(state: &mut State, px: i32, revealed: usize) -> Vec<Option<Arc<Canvas>>> {
    let open: Vec<TrayItem> = state.tray.as_ref().map_or_else(Vec::new, |t| {
        let from = t.hidden.len().saturating_sub(revealed);
        t.hidden[from..].to_vec()
    });
    let items: Vec<TrayItem> = state
        .tray
        .as_ref()
        .map(|t| t.shown().to_vec())
        .unwrap_or_default();
    let (lead, trail): (Vec<_>, Vec<_>) =
        builtin::shown(state).into_iter().partition(|b| b.leads());
    let own = |state: &mut State, b| builtin::icon(state, b, px).map(Arc::new);
    let mut icons: Vec<Option<Arc<Canvas>>> =
        open.iter().map(|it| item_icon(state, it, px)).collect();
    icons.extend(lead.into_iter().map(|b| own(state, b)));
    icons.extend(items.iter().map(|it| item_icon(state, it, px)));
    icons.extend(trail.into_iter().map(|b| own(state, b)));
    icons
}

pub fn invalidate_icons(state: &mut State) {
    if let Some(t) = state.tray.as_mut() {
        t.icon_cache.clear();
    }
}

fn cell_rect(state: &State, bar: TreeId, index: usize) -> Option<(Output, Rect)> {
    let b = state.bar(bar)?;
    let r = *b.layout.tray.get(index)?;
    let abs = bar_cell(state.bar_placement(&b.output).shown, r);
    Some((b.output.clone(), abs))
}

pub fn bell_cell(state: &State, output: &Output) -> Option<Rect> {
    let k = builtin::shown(state)
        .iter()
        .position(|b| *b == builtin::Builtin::Notifications)?;
    let bar = state.bars.iter().find(|b| b.output == *output)?;
    let i =
        (0..bar.layout.tray.len()).find(|&i| cell_at(state, bar.tree, i) == Cell::Builtin(k))?;
    cell_rect(state, bar.tree, i).map(|(_, r)| r)
}

pub(crate) fn bar_cell(shown: Rect, r: Rect) -> Rect {
    Rect::new(shown.x + r.x, shown.y + r.y, r.width, r.height)
}

pub fn click(state: &mut State, bar: TreeId, index: usize, button: u32) {
    let Some((output, cell)) = cell_rect(state, bar, index) else {
        return;
    };
    let leading = leading(state);
    let revealed = state.bar(bar).map_or(0, |b| b.layout.tray_hidden);
    if let Cell::Builtin(b) = cell_at(state, bar, index) {
        if let Some(&b) = builtin::shown(state).get(b) {
            let open_here = builtin::menu_open(state, b);
            state.close_menu();
            if !open_here {
                builtin::click(state, &output, b, cell, button);
            }
        }
        return;
    }
    crate::notifycenter::close(state);
    let Some(id) = state
        .tray
        .as_ref()
        .and_then(|t| t.item_at(revealed, leading, index))
        .map(|i| i.id.clone())
    else {
        return;
    };
    let at = state.bar_placement(&output).click_point(cell);
    click_item(state, output, id, cell, at, button);
}

/// A pointer button on item `id`, whose icon sits at `icon` (layout
/// coordinates) on `output`'s bar or flyout: left activates it, middle is
/// its secondary activation, right opens its menu (or asks it for one).
/// The item hears of a click at `at`, on the bar's or flyout's edge
/// facing the screen.
pub fn click_item(
    state: &mut State,
    output: Output,
    id: String,
    icon: Rect,
    at: (i32, i32),
    button: u32,
) {
    let open_here = matches!(
        state.menus.first().map(|m| &m.kind),
        Some(MenuKind::Tray { id: open, .. }) if *open == id
    );
    if !state.menus.is_empty() {
        state.close_menu();
    }
    if open_here {
        return;
    }
    let has_menu = state
        .tray
        .as_ref()
        .and_then(|t| t.item(&id))
        .is_some_and(|i| i.menu_path.is_some());
    let Some(command) = click_command(id.clone(), at, button, has_menu, || {
        state.new_activation_token()
    }) else {
        return;
    };
    if matches!(command, TrayCommand::GetLayout { .. })
        && let Some(tray) = state.tray.as_mut()
    {
        tray.pending_menu = Some((id, output, icon, at));
        tray.submenus.clear();
    }
    send(state, command);
}

/// An activation carries a token from `mint`, so the window it shows or
/// raises can take focus.
fn click_command(
    id: String,
    at: (i32, i32),
    button: u32,
    has_menu: bool,
    mint: impl FnOnce() -> String,
) -> Option<TrayCommand> {
    let (x, y) = at;
    let kind = match button {
        BTN_LEFT => ClickKind::Activate,
        BTN_MIDDLE => ClickKind::SecondaryActivate,
        BTN_RIGHT if has_menu => return Some(TrayCommand::GetLayout { id, parent: 0 }),
        BTN_RIGHT => ClickKind::ContextMenu,
        _ => return None,
    };
    let token = (kind != ClickKind::ContextMenu).then(mint);
    Some(TrayCommand::Click {
        id,
        x,
        y,
        kind,
        token,
    })
}

/// A wheel step on tray cell `index`: a built-in cell's goes to
/// [`builtin::scroll`], an item's straight to the item as `Scroll`, leaving
/// an open menu alone. `discrete` and `continuous` are the axis event's (see
/// [`wheel_steps`]). An item's sub-notch remainder is kept for the next event
/// over the same cell and axis; the displays cell keeps its own in
/// [`crate::displays::Displays::wheel`].
pub fn scroll(
    state: &mut State,
    bar: TreeId,
    index: usize,
    horizontal: bool,
    discrete: i32,
    continuous: f64,
) {
    if let Cell::Builtin(k) = cell_at(state, bar, index) {
        if let Some(&b) = builtin::shown(state).get(k) {
            builtin::scroll(state, b, horizontal, discrete, continuous);
        }
        return;
    }
    let leading = leading(state);
    let revealed = state.bar(bar).map_or(0, |b| b.layout.tray_hidden);
    let Some(tray) = state.tray.as_mut() else {
        return;
    };
    // A remainder belongs to the cell and the axis it came from; moving to
    // another icon starts the count again.
    let carried = match tray.wheel {
        Some((cell, h, r)) if cell == index && h == horizontal => r,
        _ => 0,
    };
    let (steps, residual) = wheel_steps(discrete, continuous, carried);
    tray.wheel = Some((index, horizontal, residual));
    let Some(id) = tray.item_at(revealed, leading, index).map(|i| i.id.clone()) else {
        return;
    };
    scroll_item(state, id, steps, horizontal);
}

pub fn wheel_left(state: &mut State) {
    if let Some(t) = state.tray.as_mut() {
        t.wheel = None;
    }
    state.displays.wheel = (0, 0.0);
}

/// Whether tray cell `index` on `bar` is the displays cell.
pub fn is_displays_cell(state: &State, bar: TreeId, index: usize) -> bool {
    let Cell::Builtin(k) = cell_at(state, bar, index) else {
        return false;
    };
    builtin::shown(state).get(k) == Some(&builtin::Builtin::Displays)
}

impl State {
    pub fn tray_axis(&mut self, frame: &AxisFrame) -> bool {
        let hit = self.scene_hit(self.pointer_location());
        // Over the history popup the wheel scrolls its list, in whole
        // notches (`wheel_steps` counts up as positive).
        if hit == Hit::NotifyCenter {
            let discrete = frame.v120.map_or(0, |v| v.1);
            let continuous = frame.axis.1;
            if discrete != 0 || continuous != 0.0 {
                let carried = self.notify.center_wheel;
                let (steps, residual) = wheel_steps(discrete, continuous, carried);
                self.notify.center_wheel = residual;
                crate::notifycenter::scroll(self, -i64::from(steps));
            }
            return true;
        }
        let (on_bar, in_flyout) = match hit {
            Hit::Taskbar {
                bar,
                hit: TaskbarHit::Tray(i),
            } => (Some((bar, i)), None),
            Hit::TrayFlyout { cell } => (None, Some(cell)),
            _ => {
                wheel_left(self);
                return false;
            }
        };
        let v120 = frame.v120.unwrap_or((0, 0));
        for (discrete, continuous, horizontal) in
            [(v120.1, frame.axis.1, false), (v120.0, frame.axis.0, true)]
        {
            if discrete == 0 && continuous == 0.0 {
                continue;
            }
            match (on_bar, in_flyout) {
                (Some((bar, i)), _) => scroll(self, bar, i, horizontal, discrete, continuous),
                (_, Some(Some(cell))) => {
                    crate::trayflyout::scroll(self, cell, horizontal, discrete, continuous)
                }
                _ => {}
            }
        }
        true
    }
}

/// Whole wheel notches an axis event carries, positive away from the user,
/// plus the 120ths left over for the next event.
///
/// Wayland counts a wheel step positive towards the user (down, or right),
/// with `discrete` in 120ths of a notch; a touchpad leaves it 0 and only the
/// sign of `continuous` counts. `Scroll` on the item side is Qt's angle
/// delta, positive away from the user, so the sign flips here and the
/// sub-notch remainder is carried in `residual`.
pub fn wheel_steps(discrete: i32, continuous: f64, residual: i32) -> (i32, i32) {
    if discrete != 0 {
        let total = residual.saturating_add(discrete);
        return (-(total / 120), total % 120);
    }
    if continuous > 0.0 {
        (-1, 0)
    } else if continuous < 0.0 {
        (1, 0)
    } else {
        (0, residual)
    }
}

pub fn scroll_item(state: &mut State, id: String, delta: i32, horizontal: bool) {
    if delta == 0 {
        return;
    }
    let orientation = if horizontal {
        Orientation::Horizontal
    } else {
        Orientation::Vertical
    };
    send(
        state,
        TrayCommand::Scroll {
            id,
            delta,
            orientation,
        },
    );
}

fn open_layout(state: &mut State, id: String, parent: i32, nodes: Vec<MenuNode>) {
    let (items, node_ids) = menu_items(&nodes);
    let pending_root = state.tray.as_mut().and_then(|t| {
        t.pending_menu
            .take_if(|(pid, ..)| *pid == id && parent == 0)
    });
    if let Some((_, output, icon, (x, y))) = pending_root {
        if !state.scene.outputs().contains(&output) || state.layout_box(Some(&output)).width <= 0 {
            return;
        }
        // The D-Bus side answers an unreadable menu with no rows: the
        // item gets `ContextMenu`, as one without a menu does. No menu
        // follows, so the bar the request held may hide again.
        if items.is_empty() {
            let kind = ClickKind::ContextMenu;
            let token = None;
            send(
                state,
                TrayCommand::Click {
                    id,
                    x,
                    y,
                    kind,
                    token,
                },
            );
            crate::autohideview::poke(state);
            return;
        }
        let usable = state.usable_area(Some(&output));
        let m = &state.config.theme.metrics;
        let style = MenuStyle::context_for(&state.config.theme, &items).fit(
            &state.text,
            &items,
            state.config.theme.fonts.size_px,
            usable.width,
        );
        let layout = MenuLayout::compute_styled(&style, &items, submenu_max_rows(m, usable.height));
        let (x, y) = crate::barplace::bar_popup(
            &state.bar_placement(&output),
            icon,
            (layout.width, layout.height),
            usable,
        );
        state.close_menu();
        state.push_menu(
            0,
            (x, y),
            layout,
            items,
            output,
            MenuKind::Tray { id, node_ids },
            MenuFrom::Bar,
        );
        state.redraw_menu(0);
        return;
    }
    let Some((depth, row)) = state
        .tray
        .as_mut()
        .and_then(|t| t.submenus.accept(&id, parent))
    else {
        return;
    };
    let Some(parent_menu) = state.menus.get(depth) else {
        return;
    };
    if !matches!(&parent_menu.kind, MenuKind::Tray { id: pid, .. } if *pid == id) {
        return;
    }
    let parent_rect = Rect::new(
        parent_menu.x,
        parent_menu.y,
        parent_menu.layout.width,
        parent_menu.layout.height,
    );
    let Some(&local_row) = parent_menu.layout.rows.get(row) else {
        return;
    };
    let row_rect = Rect::new(
        parent_menu.x + local_row.x,
        parent_menu.y + local_row.y,
        local_row.width,
        local_row.height,
    );
    let output = parent_menu.output.clone();
    let usable = state.usable_area(Some(&output));
    let m = &state.config.theme.metrics;
    let style = MenuStyle::context_for(&state.config.theme, &items).fit(
        &state.text,
        &items,
        state.config.theme.fonts.size_px,
        usable.width,
    );
    let layout = MenuLayout::compute_styled(&style, &items, submenu_max_rows(m, usable.height));
    let (x, y) = place_submenu(parent_rect, row_rect, (layout.width, layout.height), usable);
    state.close_menus_above(depth);
    state.push_menu(
        depth + 1,
        (x, y),
        layout,
        items,
        output,
        MenuKind::Tray { id, node_ids },
        MenuFrom::Bar,
    );
    if let Some(m) = state.menus.last_mut() {
        m.parent_row = Some(row);
    }
    if let Some(tray) = state.tray.as_mut() {
        tray.submenus.opened(depth, row);
    }
    state.redraw_menu(depth + 1);
}

pub fn menu_hover(state: &mut State, depth: usize, idx: Option<usize>) {
    let Some(menu) = state.menus.get(depth) else {
        return;
    };
    let MenuKind::Tray { id, node_ids } = &menu.kind else {
        return;
    };
    let Some(i) = idx else { return };
    let is_submenu = matches!(
        menu.items.get(i),
        Some(MenuItem::Entry {
            submenu: true,
            enabled: true,
            ..
        })
    );
    if !is_submenu {
        state.close_menus_above(depth);
        return;
    }
    let (id, node) = (id.clone(), node_ids.get(i).copied().flatten());
    let Some(node) = node else { return };
    let Some(tray) = state.tray.as_mut() else {
        return;
    };
    if !tray.submenus.needs_request(&id, depth, i) {
        return;
    }
    tray.submenus.request(SubmenuRequest {
        id: id.clone(),
        node,
        depth,
        row: i,
    });
    tray.send(TrayCommand::GetLayout { id, parent: node });
}

pub fn forget_output(state: &mut State) {
    let outputs = state.scene.outputs().to_vec();
    if let Some(tray) = state.tray.as_mut()
        && tray
            .pending_menu
            .as_ref()
            .is_some_and(|(_, o, ..)| !outputs.contains(o))
    {
        tray.pending_menu = None;
    }
}

/// Every menu closed: nothing in flight can still be wanted. A dropped
/// root request no longer holds its bar, so the bars hear of it.
pub fn menus_closed(state: &mut State) {
    let Some(tray) = state.tray.as_mut() else {
        return;
    };
    let dropped = tray.pending_menu.take().is_some();
    tray.submenus.clear();
    if dropped {
        crate::autohideview::poke(state);
    }
}

pub fn menus_closed_above(state: &mut State, depth: usize) {
    if let Some(tray) = state.tray.as_mut() {
        tray.submenus.closed_above(depth);
    }
}

pub fn menu_activate(state: &mut State, depth: usize, idx: usize) {
    let Some(menu) = state.menus.get(depth) else {
        return;
    };
    let MenuKind::Tray { id, node_ids } = &menu.kind else {
        return;
    };
    let enabled = matches!(
        menu.items.get(idx),
        Some(MenuItem::Entry {
            enabled: true,
            submenu: false,
            ..
        })
    );
    let node = node_ids.get(idx).copied().flatten();
    let id = id.clone();
    if let (true, Some(item)) = (enabled, node) {
        state.close_menu();
        let token = state.new_activation_token();
        send(state, TrayCommand::MenuEvent { id, item, token });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_remembered_icon_file_is_never_read_on_the_loop() {
        let file = "/srv/icons/tray.png";
        let mut icons = IconCache::new(crate::icontheme::IconTheme::load("none", &[]));
        assert!(past_icon(&mut icons, Some(file)).is_empty());
        assert_eq!(
            icons.take_wanted(),
            [(file.to_string(), 16), (file.to_string(), 32)]
        );
        icons.put_file(file, 16, Some(Canvas::new(16, 16)));
        icons.put_file(file, 32, None);
        let widths: Vec<u32> = past_icon(&mut icons, Some(file))
            .iter()
            .map(|s| s.width)
            .collect();
        assert_eq!(widths, [16]);
    }

    #[test]
    fn the_balloon_icon_is_the_picture_else_the_app_icon() {
        let src = |w: u32| IconSource {
            width: w,
            height: w,
            pixels: Vec::new(),
        };
        let mut n = Notification::new(1, "s".into(), "b".into());
        n.app_icon = vec![src(16)];
        assert_eq!(n.balloon_icon(), &n.app_icon[..]);
        n.image = vec![src(32)];
        assert_eq!(n.balloon_icon(), &n.image[..]);
    }

    use super::*;
    use crate::startmenu::MenuItem;

    #[test]
    fn a_tray_cell_is_placed_from_the_bars_own_strip() {
        let right = crate::barplace::place(
            Rect::new(0, 0, 1920, 1080),
            reprisekit::taskbar::Edge::Right,
            60,
        );
        assert_eq!(
            bar_cell(right.shown, Rect::new(4, 1000, 16, 16)),
            Rect::new(1864, 1000, 16, 16),
            "a right bar's cell is right of the output's left edge"
        );
        let bottom = crate::barplace::place(
            Rect::new(1920, 0, 1280, 720),
            reprisekit::taskbar::Edge::Bottom,
            30,
        );
        assert_eq!(
            bar_cell(bottom.shown, Rect::new(1200, 7, 16, 16)),
            Rect::new(3120, 697, 16, 16)
        );
    }

    #[test]
    fn a_click_reaches_the_item_at_the_cells_centre_on_the_bars_screen_side_edge() {
        use reprisekit::taskbar::Edge;
        let screen = Rect::new(0, 0, 1920, 1080);
        let bottom = crate::barplace::place(screen, Edge::Bottom, 30);
        let cell = Rect::new(1841, 1057, 16, 16);
        assert_eq!(
            bottom.click_point(cell),
            (1849, 1050),
            "a bottom bar's top edge"
        );
        let top = crate::barplace::place(screen, Edge::Top, 30);
        let cell = Rect::new(1841, 7, 16, 16);
        assert_eq!(top.click_point(cell), (1849, 30), "a top bar's foot");
        let left = crate::barplace::place(screen, Edge::Left, 60);
        let cell = Rect::new(22, 1000, 16, 16);
        assert_eq!(
            left.click_point(cell),
            (60, 1008),
            "a left bar's right edge"
        );
    }

    #[test]
    fn an_item_hears_a_click_at_the_point_it_is_given() {
        for (button, has_menu, want) in [
            (BTN_LEFT, true, ClickKind::Activate),
            (BTN_MIDDLE, true, ClickKind::SecondaryActivate),
            (BTN_RIGHT, false, ClickKind::ContextMenu),
        ] {
            match click_command("a".into(), (1849, 1050), button, has_menu, || "t".into()) {
                Some(TrayCommand::Click { id, x, y, kind, .. }) => assert_eq!(
                    (id.as_str(), x, y, kind),
                    ("a", 1849, 1050, want),
                    "{want:?} is heard at (1849, 1050)"
                ),
                other => panic!("{want:?}: a click at (1849, 1050), not {other:?}"),
            }
        }
        assert!(
            matches!(
                click_command("a".into(), (1849, 1050), BTN_RIGHT, true, || "t".into()),
                Some(TrayCommand::GetLayout { parent: 0, .. })
            ),
            "a right click on an item with a menu asks for the menu's layout"
        );
        assert!(
            click_command("a".into(), (1849, 1050), 0x113, false, || "t".into()).is_none(),
            "a side button asks nothing"
        );
    }

    #[test]
    fn an_activation_carries_a_token_and_a_context_menu_none() {
        for (button, want) in [
            (BTN_LEFT, Some("t")),
            (BTN_MIDDLE, Some("t")),
            (BTN_RIGHT, None),
        ] {
            match click_command("a".into(), (0, 0), button, false, || "t".into()) {
                Some(TrayCommand::Click { token, .. }) => {
                    assert_eq!(
                        token.as_deref(),
                        want,
                        "button {button:#x} carries {want:?}"
                    )
                }
                other => panic!("button {button:#x}: a click, not {other:?}"),
            }
        }
    }

    #[test]
    fn clip_cuts_on_a_char_boundary() {
        assert_eq!(clip("héllo".into(), 2), "hé");
        assert_eq!(clip("hi".into(), 5), "hi");
        assert_eq!(
            clip("x".repeat(1000), BODY_CHARS).chars().count(),
            BODY_CHARS
        );
    }

    fn request(id: &str, node: i32, depth: usize, row: usize) -> SubmenuRequest {
        SubmenuRequest {
            id: id.into(),
            node,
            depth,
            row,
        }
    }

    #[test]
    fn submenu_reply_is_accepted_only_for_the_node_still_wanted() {
        let mut s = Submenus::default();
        s.request(request("a", 5, 0, 2));
        // A reply for the row the pointer has already left: dropped, and the
        // request in flight survives it.
        assert_eq!(s.accept("a", 9), None);
        assert_eq!(s.accept("b", 5), None);
        assert_eq!(s.accept("a", 5), Some((0, 2)));
        assert_eq!(s.accept("a", 5), None);
    }

    #[test]
    fn hovering_does_not_re_request_an_open_or_in_flight_submenu() {
        let mut s = Submenus::default();
        assert!(s.needs_request("a", 0, 2));
        s.request(request("a", 5, 0, 2));
        assert!(!s.needs_request("a", 0, 2));
        assert!(s.needs_request("a", 0, 3));
        s.accept("a", 5);
        s.opened(0, 2);
        assert!(!s.needs_request("a", 0, 2));
        assert!(s.needs_request("a", 1, 0));
    }

    #[test]
    fn closing_menus_drops_the_requests_they_belong_to() {
        let mut s = Submenus::default();
        s.request(request("a", 5, 0, 2));
        s.accept("a", 5);
        s.opened(0, 2);
        s.request(request("a", 7, 1, 0));
        s.closed_above(0);
        assert_eq!(s.pending, None);
        assert!(s.needs_request("a", 0, 2));

        s.request(request("a", 5, 0, 2));
        s.clear();
        assert_eq!(s, Submenus::default());

        s.request(request("a", 5, 0, 2));
        s.forget_item("b");
        assert!(s.pending.is_some());
        s.forget_item("a");
        assert_eq!(s.pending, None);
    }

    #[test]
    fn strip_mnemonic_drops_single_underscores_and_keeps_doubles() {
        assert_eq!(strip_mnemonic("_File"), "File");
        assert_eq!(strip_mnemonic("Save _As..."), "Save As...");
        assert_eq!(strip_mnemonic("100__percent"), "100_percent");
        assert_eq!(strip_mnemonic("plain"), "plain");
        assert_eq!(strip_mnemonic(""), "");
    }

    fn merge(bar: &mut Vec<TrayItem>, hidden: &mut Vec<TrayItem>, it: TrayItem) -> bool {
        let place = placement(&it, Behaviour::Auto, true);
        merge_item(bar, hidden, it, place)
    }

    fn item(id: &str, passive: bool) -> TrayItem {
        TrayItem {
            id: id.into(),
            key: id.into(),
            service: ":1.1".into(),
            path: "/StatusNotifierItem".into(),
            title: String::new(),
            tooltip: String::new(),
            icon_name: None,
            icon_theme_path: None,
            pixmaps: Arc::from([]),
            named: Arc::from([]),
            menu_path: None,
            passive,
            pid: None,
        }
    }

    #[test]
    fn cells_past_the_shown_items_are_builtin() {
        assert_eq!(Cell::at(0, 3, 0, 2), Cell::Item(2));
        assert_eq!(Cell::at(0, 3, 0, 3), Cell::Builtin(0));
        assert_eq!(
            Cell::at(0, MAX_ITEMS + 4, 0, MAX_ITEMS - 1),
            Cell::Item(MAX_ITEMS - 1)
        );
        assert_eq!(Cell::at(0, MAX_ITEMS + 4, 0, MAX_ITEMS), Cell::Builtin(0));
        assert_eq!(
            Cell::at(0, MAX_ITEMS + 4, 0, MAX_ITEMS + 1),
            Cell::Builtin(1)
        );
    }

    #[test]
    fn revealed_cells_lead_and_map_to_the_hidden_run_nearest_the_icons() {
        assert_eq!(Cell::at(2, 3, 1, 0), Cell::Hidden(0));
        assert_eq!(Cell::at(2, 3, 1, 1), Cell::Hidden(1));
        assert_eq!(Cell::at(2, 3, 1, 2), Cell::Builtin(0));
        assert_eq!(Cell::at(2, 3, 1, 3), Cell::Item(0));
        assert_eq!(Cell::at(0, 3, 1, 0), Cell::Builtin(0), "shut: as before");
    }

    #[test]
    fn a_revealed_cell_maps_to_the_hidden_items_nearest_the_icons() {
        assert_eq!(hidden_index(4, 2, 0), Some(2));
        assert_eq!(hidden_index(4, 2, 1), Some(3));
        assert_eq!(hidden_index(1, 2, 0), None, "the run shrank under the draw");
        assert_eq!(
            hidden_index(1, 2, 1),
            Some(0),
            "the cell nearest the shown icons keeps the last item"
        );
    }

    #[test]
    fn a_cut_run_draws_the_items_its_cells_click() {
        let hidden = ["a", "b", "c", "d", "e"];
        let mut icons: Vec<&str> = hidden[2..].to_vec();
        icons.push("shown");
        keep_run(&mut icons, 3, 2);
        assert_eq!(icons, ["d", "e", "shown"]);
        for (i, icon) in icons.iter().take(2).enumerate() {
            assert_eq!(Some(icon), hidden_index(5, 2, i).map(|k| &hidden[k]));
        }
    }

    #[test]
    fn a_whole_run_keeps_every_picture() {
        let mut icons = vec![1, 2, 3];
        keep_run(&mut icons, 2, 2);
        assert_eq!(icons, [1, 2, 3]);
    }

    #[test]
    fn leading_cells_come_before_the_items() {
        assert_eq!(Cell::at(0, 2, 1, 0), Cell::Builtin(0));
        assert_eq!(Cell::at(0, 2, 1, 1), Cell::Item(0));
        assert_eq!(Cell::at(0, 2, 1, 2), Cell::Item(1));
        assert_eq!(Cell::at(0, 2, 1, 3), Cell::Builtin(1));
        assert_eq!(Cell::at(0, 0, 1, 1), Cell::Builtin(1));
    }

    #[test]
    fn passive_items_are_hidden_and_the_rest_stay_on_the_bar() {
        assert_eq!(
            placement(&item("a", true), Behaviour::Auto, true),
            Place::Hidden
        );
        assert_eq!(
            placement(&item("a", false), Behaviour::Auto, true),
            Place::Bar
        );
    }

    #[test]
    fn a_choice_overrides_the_status_only_with_overflow_on() {
        let (busy, idle) = (item("a", false), item("b", true));
        assert_eq!(placement(&busy, Behaviour::Hide, true), Place::Hidden);
        assert_eq!(placement(&busy, Behaviour::Mute, true), Place::Hidden);
        assert_eq!(placement(&idle, Behaviour::Show, true), Place::Bar);
        assert_eq!(placement(&busy, Behaviour::Hide, false), Place::Bar);
        assert_eq!(placement(&idle, Behaviour::Auto, false), Place::Bar);
    }

    #[test]
    fn new_choices_move_items_and_keep_the_order_in_each_list() {
        fn ids(l: &[TrayItem]) -> Vec<&str> {
            l.iter().map(|i| i.id.as_str()).collect()
        }
        let mut bar = vec![item("a", false), item("b", false), item("c", false)];
        let mut hidden = vec![item("p", true)];
        let mut choices = BTreeMap::new();
        let keep = |b| Remembered {
            behaviour: b,
            name: String::new(),
            icon: None,
        };
        choices.insert("b".to_string(), keep(Behaviour::Hide));
        choices.insert("p".to_string(), keep(Behaviour::Show));
        arrange(&mut bar, &mut hidden, &choices, true);
        assert_eq!((ids(&bar), ids(&hidden)), (vec!["a", "c", "p"], vec!["b"]));
        arrange(&mut bar, &mut hidden, &choices, false);
        assert_eq!(
            (ids(&bar), ids(&hidden)),
            (vec!["a", "c", "p", "b"], vec![])
        );
    }

    #[test]
    fn ok_keeps_only_choices_away_from_auto_and_names_them() {
        let mut choices = BTreeMap::new();
        choices.insert(
            "gone".to_string(),
            Remembered {
                behaviour: Behaviour::Hide,
                name: "Old Applet".into(),
                icon: Some("old".into()),
            },
        );
        let mut net = item(":1.4/StatusNotifierItem", false);
        net.key = "nm-applet".into();
        net.tooltip = "Network\nConnected".into();
        net.icon_name = Some("network-wired".into());
        let picked = vec![
            ("nm-applet".to_string(), Behaviour::Show),
            ("gone".to_string(), Behaviour::Mute),
            ("other".to_string(), Behaviour::Auto),
        ];
        remember(&mut choices, &picked, &[&net]);
        assert_eq!(
            choices["nm-applet"],
            Remembered {
                behaviour: Behaviour::Show,
                name: "Network".into(),
                icon: Some("network-wired".into()),
            }
        );
        assert_eq!(choices["gone"].behaviour, Behaviour::Mute);
        assert_eq!(
            choices["gone"].name, "Old Applet",
            "a past item keeps its name"
        );
        remember(&mut choices, &[("gone".into(), Behaviour::Auto)], &[]);
        assert!(!choices.contains_key("gone"), "back to Auto: forgotten");
        assert!(!choices.contains_key("other"));
    }

    #[test]
    fn the_chevron_shows_only_with_overflow_on_and_an_item_hidden() {
        let parse = |text: &str| crate::config::Config::parse(text).unwrap();
        let off = parse("[taskbar]\ntray_overflow = false\n");
        let glass = parse("[appearance]\ninterface = \"glass\"\n");
        assert!(!shows_chevron(&off, 2), "overflow off");
        assert!(!shows_chevron(&glass, 0), "nothing hidden");
        assert!(shows_chevron(&glass, 1));
    }

    #[test]
    fn tray_overflow_follows_its_taskbar_key_in_every_look() {
        let parse = |text: &str| crate::config::Config::parse(text).unwrap();
        let glass_off =
            parse("[appearance]\ninterface = \"glass\"\n[taskbar]\ntray_overflow = false\n");
        assert!(
            !shows_chevron(&glass_off, 2),
            "off under Glass: no ^, the tray as it is without overflow"
        );
        let classic_on = parse("[taskbar]\ntray_overflow = true\n");
        assert!(
            shows_chevron(&classic_on, 1),
            "on under Classic: the ^ shows"
        );
    }

    #[test]
    fn an_item_changing_status_moves_between_the_bar_and_the_hidden_list() {
        let ids = |l: &[TrayItem]| l.iter().map(|i| i.id.clone()).collect::<Vec<_>>();
        let (mut bar, mut hidden) = (Vec::new(), Vec::new());
        merge(&mut bar, &mut hidden, item("a", true));
        merge(&mut bar, &mut hidden, item("b", false));
        merge(&mut bar, &mut hidden, item("c", false));
        assert_eq!(
            (ids(&bar), ids(&hidden)),
            (vec!["b".into(), "c".into()], vec!["a".into()])
        );
        merge(&mut bar, &mut hidden, item("b", true));
        assert_eq!(
            (ids(&bar), ids(&hidden)),
            (vec!["c".into()], vec!["a".into(), "b".into()])
        );
        merge(&mut bar, &mut hidden, item("a", false));
        assert_eq!(
            (ids(&bar), ids(&hidden)),
            (vec!["c".into(), "a".into()], vec!["b".into()])
        );
        let mut c = item("c", false);
        c.title = "new".into();
        merge(&mut bar, &mut hidden, c);
        assert_eq!(ids(&bar), vec!["c".to_string(), "a".into()]);
        assert_eq!(bar[0].title, "new");
    }

    #[test]
    fn only_a_change_to_what_the_cell_draws_redraws_the_tray() {
        let (mut bar, mut hidden) = (Vec::new(), Vec::new());
        assert!(merge(&mut bar, &mut hidden, item("a", false)), "a new item");
        let mut a = item("a", false);
        a.tooltip = "Battery 80%".into();
        a.title = "Power".into();
        a.menu_path = Some("/Menu".into());
        a.pid = Some(42);
        assert!(
            !merge(&mut bar, &mut hidden, a.clone()),
            "tooltip, title, menu and pid are not drawn"
        );
        assert_eq!(bar[0].tooltip, "Battery 80%", "the model still takes them");
        let red = IconSource {
            width: 1,
            height: 1,
            pixels: vec![0xFFFF_0000],
        };
        a.pixmaps = Arc::from([red.clone()]);
        assert!(merge(&mut bar, &mut hidden, a.clone()), "a new pixmap");
        a.pixmaps = Arc::from([red]);
        assert!(
            !merge(&mut bar, &mut hidden, a.clone()),
            "the same pixels in a new Arc"
        );
        a.icon_name = Some("battery-low".into());
        assert!(merge(&mut bar, &mut hidden, a.clone()), "an icon name");
        a.icon_theme_path = Some("/opt/icons".into());
        assert!(merge(&mut bar, &mut hidden, a.clone()), "a theme path");
        a.named = Arc::from([IconSource {
            width: 1,
            height: 1,
            pixels: vec![0xFF00_FF00],
        }]);
        assert!(merge(&mut bar, &mut hidden, a.clone()), "a decoded name");
        a.passive = true;
        assert!(merge(&mut bar, &mut hidden, a), "Passive: off the bar");
    }

    #[test]
    fn pixmap_to_source_reads_big_endian_argb_and_premultiplies() {
        let bytes = [
            0xFF, 0xFF, 0x00, 0x00, // A R G B
            0x80, 0xFF, 0xFF, 0xFF,
        ];
        let src = pixmap_to_source(2, 1, &bytes).unwrap();
        assert_eq!((src.width, src.height), (2, 1));
        assert_eq!(src.pixels[0], 0xFFFF_0000);
        assert_eq!(src.pixels[1], reprisekit::icon::premultiply(0x80FF_FFFF));
    }

    #[test]
    fn pixmap_to_source_rejects_short_or_empty_buffers() {
        assert!(pixmap_to_source(2, 1, &[0; 4]).is_none());
        assert!(pixmap_to_source(0, 0, &[]).is_none());
        assert!(pixmap_to_source(-1, 1, &[0; 4]).is_none());
    }

    #[test]
    fn pixmap_to_source_rejects_oversized_claims_regardless_of_buffer_len() {
        // A hostile peer claiming 60000x60000 must be rejected before the
        // `w * h * 4` byte length is even computed, whether the buffer it
        // sent is short (a few bytes) or actually that large.
        assert!(pixmap_to_source(60_000, 60_000, &[0; 16]).is_none());
        let huge = vec![0u8; 60_000 * 4];
        assert!(pixmap_to_source(60_000, 1, &huge).is_none());
    }

    #[test]
    fn item_pixmaps_keeps_at_most_four_entries_of_at_most_256_px() {
        let pixels = vec![0u8; 1024 * 1024 * 4];
        let entry = |side: i32| (side, side, &pixels[..(side * side * 4) as usize]);
        let mut raw: Vec<_> = (0..30).map(|_| entry(1024)).collect();
        raw.extend([16, 20, 22, 24, 32, 48, 64, 256, 300].map(entry));
        let kept = item_pixmaps(&raw);
        let widths: Vec<u32> = kept.iter().map(|p| p.width).collect();
        assert_eq!(widths, [16, 22, 32, 48], "one entry per tray size");

        let big: Vec<_> = (0..30).map(|_| entry(256)).collect();
        assert_eq!(item_pixmaps(&big).len(), 1, "one 256 px entry out of 30");
        assert!(
            item_pixmaps(&[entry(1024)]).is_empty(),
            "1024 px is dropped"
        );
    }

    fn node(id: i32, label: &str) -> MenuNode {
        MenuNode {
            id,
            label: label.into(),
            separator: false,
            enabled: true,
            visible: true,
            checked: None,
            radio: false,
            has_children: false,
            default: false,
        }
    }

    #[test]
    fn menu_items_maps_nodes_and_skips_invisible() {
        let nodes = vec![
            node(1, "_Connect"),
            MenuNode {
                separator: true,
                ..node(2, "")
            },
            MenuNode {
                visible: false,
                ..node(3, "hidden")
            },
            MenuNode {
                enabled: false,
                checked: Some(true),
                has_children: true,
                ..node(4, "More")
            },
            MenuNode {
                default: true,
                ..node(5, "Open")
            },
            MenuNode {
                checked: Some(true),
                radio: true,
                ..node(6, "Balanced")
            },
        ];
        let (items, ids) = menu_items(&nodes);
        assert_eq!(items.len(), 5);
        assert_eq!(ids, vec![Some(1), None, Some(4), Some(5), Some(6)]);
        assert_eq!(items[0], MenuItem::entry("Connect"));
        assert_eq!(items[1], MenuItem::Separator);
        assert_eq!(
            items[2],
            MenuItem::entry("More")
                .enabled(false)
                .checked(true)
                .submenu(true)
        );
        assert_eq!(
            items[3],
            MenuItem::entry("Open").bold(true),
            "the default row is bold"
        );
        assert_eq!(
            items[4],
            MenuItem::entry("Balanced").checked(true).radio(true),
            "a radio row is bulleted"
        );
    }
}
