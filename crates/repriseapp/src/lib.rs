//! Wayland client host for reprisekit programs: an [`App`] draws into a [`Canvas`] and takes
//! [`Event`]s; `run` gives each window an xdg toplevel and a `wl_shm` buffer.

pub mod about;
mod access;
pub mod decode;
pub mod dnd;
mod editmenu;
pub mod fielddrop;
pub mod filedlg;
mod hangup;
pub mod heads;
pub mod keys;
pub mod listicons;
pub mod listjob;
mod menupopup;
pub mod oneshot;
pub mod openwith;
mod outputmgmt;
mod primary;
mod tabgroup;
mod textinput;
pub mod thumbs;
pub mod unlock;

use smithay_client_toolkit::reexports::calloop::channel as reprise_channel;
use smithay_client_toolkit::reexports::protocols::ext::background_effect::v1::client::{
    ext_background_effect_manager_v1, ext_background_effect_surface_v1,
};
use smithay_client_toolkit::reexports::protocols::wp::{
    fractional_scale::v1::client::{wp_fractional_scale_manager_v1, wp_fractional_scale_v1},
    keyboard_shortcuts_inhibit::zv1::client::{
        zwp_keyboard_shortcuts_inhibit_manager_v1, zwp_keyboard_shortcuts_inhibitor_v1,
    },
    text_input::zv3::client::{zwp_text_input_manager_v3, zwp_text_input_v3},
    viewporter::client::{wp_viewport, wp_viewporter},
};
use smithay_client_toolkit::reexports::protocols::xdg::{
    dialog::v1::client::{xdg_dialog_v1, xdg_wm_dialog_v1},
    foreign::{
        zv1::client::{zxdg_imported_v1, zxdg_importer_v1},
        zv2::client::{zxdg_imported_v2, zxdg_importer_v2},
    },
    shell::client::xdg_toplevel,
    toplevel_tag::v1::client::xdg_toplevel_tag_manager_v1,
};
use smithay_client_toolkit::{
    activation::{ActivationHandler, ActivationState, RequestData},
    compositor::{CompositorHandler, CompositorState, FrameCallbackData},
    data_device_manager::{
        DataDeviceManagerState, ReadPipe, WritePipe,
        data_device::{DataDevice, DataDeviceHandler},
        data_offer::{DataOfferHandler, DragOffer, SelectionOffer},
        data_source::{CopyPasteSource, DataSourceHandler, DragSource},
    },
    delegate_dispatch2, delegate_registry,
    dispatch2::Dispatch2,
    output::{OutputHandler, OutputState},
    reexports::calloop::{
        EventLoop, LoopHandle, PostAction,
        channel::{Sender, channel},
        ping::{Ping, make_ping},
        timer::{TimeoutAction, Timer},
    },
    reexports::calloop_wayland_source::WaylandSource,
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        xdg::{
            XdgShell,
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
        },
    },
    shm::{
        Shm, ShmHandler,
        slot::{Buffer, SlotPool},
    },
};
use std::cell::RefCell;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use wayland_client::{
    Connection, Proxy, QueueHandle,
    globals::registry_queue_init,
    protocol::{
        wl_data_device::WlDataDevice, wl_data_device_manager::DndAction,
        wl_data_offer::WlDataOffer, wl_data_source::WlDataSource, wl_keyboard, wl_output,
        wl_pointer, wl_seat, wl_shm, wl_surface,
    },
};

pub use access::{Reader, ReaderForm};
pub use dnd::{Accept, Actions, DragAction, DragOut, DropKind, Dropped, Offered, Uri};
pub use fielddrop::{DropForm, FileDrop};
use reprisefs::pipe::PasteBuf;
pub use reprisekit;
use reprisekit::access::{Gesture, Reveal, Route, Tree, accesskit};
use reprisekit::controls::form;
use reprisekit::controls::menu::MenuCtx;
use reprisekit::cues::Cues;
use reprisekit::icontheme::{IconCache, IconTheme};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::textedit::{EditKey, TextSpot, key_for};
use reprisekit::theme::Theme;
use tabgroup::reprise_tabs_v1;
use zeroize::{Zeroize, Zeroizing};

pub struct Ctx {
    /// The look's theme. Its text stays at 100% unless the program is
    /// [`App::text_sized`]: then it is at the text size, and the program lays
    /// its lists, menus and bars out with [`Theme::controls`]. Forms draw with
    /// [`Ctx::forms`].
    pub theme: Theme,
    /// `[accessibility] text_size`, in percent: what a program grows its
    /// forms ([`Ctx::forms`]) and its document's text (the edit control) by.
    pub text_size: u32,
    /// The look's UI font at 100%, shared: a program holding a clone for its dialogs takes it
    /// again from here, as a `[fonts]` change replaces it.
    pub text: Rc<TextRenderer>,
    pub icons: IconCache,
    pub caret_on: bool,
    /// The window's client size, logical px.
    pub size: (i32, i32),
    pub maximized: bool,
    /// The window is at the size its program picked, held inside the compositor's bounds:
    /// neither the user nor the compositor sized it since. A program keeps only other sizes.
    pub at_pick: bool,
    /// The global position of the window the event or draw is for: the
    /// output's top-left for a flyout, (0, 0) for a toplevel, which
    /// cannot know its own.
    pub origin: (i32, i32),
    /// Wakes the event loop with `Event::Wake` on `MAIN` from any thread.
    pub wake: Waker,
    pub heads: heads::HeadRequests,
    /// Set while handling an event by a control that steps on its own while held (a sheet's
    /// scrollbar arrow, [`keys::sheet_input`]): the host sends the window an `Event::Tick`.
    pub want_tick: std::cell::Cell<bool>,
}

impl Ctx {
    /// The theme a program's forms, property sheets, wizards and message boxes draw with: `theme`
    /// at the text size. Their 100% layouts grow by its `text`; a window holding one is
    /// `forms().text.size` of its 100% size.
    pub fn forms(&self) -> Theme {
        if self.theme.text.is_one() {
            self.theme.with_form_text(self.text_size)
        } else {
            self.theme.clone()
        }
    }

    /// How [`Ctx::forms`] grows a 100% layout, without building its theme.
    pub fn form_units(&self) -> reprisekit::theme::TextUnits {
        let base = self.base_px();
        reprisekit::theme::TextUnits::new(reprisekit::theme::text_px(base, self.text_size), base)
    }

    /// Point the window fields at slot `s`'s window, for an event, draw or tree of it.
    fn aim(&mut self, s: &Slot) {
        self.size = s.size;
        self.maximized = s.maximized;
        self.at_pick = at_pick(s.size, s.picked, s.bounds);
        self.origin = s.origin;
    }

    /// The UI font's px at 100% text, what a form's `units().px(..)` grows.
    pub fn base_px(&self) -> u32 {
        self.theme.text.base_px(self.theme.fonts.size_px)
    }

    /// Ask the compositor to set `cfg` on output `name`; returns the number
    /// `Event::HeadsApplied` answers with. Fails without
    /// [`App::output_management`] or the protocol.
    pub fn configure_head(&mut self, name: &str, cfg: heads::HeadConfig) -> u32 {
        self.heads.configure(name, cfg)
    }

    /// Run `draw` with the look's style built for `scale` device px per
    /// logical px, so parts land crisp on a canvas at `scale`. The theme's
    /// metrics stay at 1: a program lays out in logical px.
    pub fn at_scale<T>(&mut self, scale: i32, draw: impl FnOnce(&Ctx) -> T) -> T {
        let look = self.theme.look.at_scale(scale);
        let rest = std::mem::replace(&mut self.theme.look, look);
        let out = draw(self);
        self.theme.look = rest;
        out
    }

    /// Whether the theme's look draws parts under the pointer apart: a
    /// host redraws for a hover change only then.
    pub fn styled(&self) -> bool {
        self.theme.look.lights_pointer()
    }
}

/// A cheap, cloneable handle that wakes the event loop: `Waker::wake`
/// (callable from any thread, e.g. a `spawn`ed helper's watcher) queues
/// `Event::Wake` on `MAIN`.
#[derive(Clone)]
pub struct Waker(Arc<Ping>);

impl Waker {
    pub fn wake(&self) {
        self.0.ping();
    }

    pub fn noop() -> Waker {
        let (ping, _source) = make_ping().expect("ping");
        Waker(Arc::new(ping))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyInput {
    /// The X keysym (see `reprisekit::textedit::keysym`); 0 (NoSymbol)
    /// for an input method's commit (its text in `utf8` too) or delete.
    pub sym: u32,
    pub utf8: Option<String>,
    pub mods: Mods,
    pub edit: Option<EditKey>,
}

/// A key may be a password's character, so its text is zeroized once the
/// program is done with it.
impl Drop for KeyInput {
    fn drop(&mut self) {
        self.utf8.zeroize();
        if let Some(EditKey::Insert(s)) = &mut self.edit {
            s.zeroize();
        }
    }
}

pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointerInput {
    Motion {
        x: i32,
        y: i32,
    },
    /// `button` is the evdev code (0x110 left, 0x111 right, 0x112 middle).
    Press {
        x: i32,
        y: i32,
        button: u32,
        time: u32,
        mods: Mods,
    },
    Release {
        x: i32,
        y: i32,
        button: u32,
        mods: Mods,
    },
    /// Vertical scroll in rows (positive is down), `reprisekit::pointer::WHEEL_ROWS`
    /// per wheel notch.
    Scroll {
        x: i32,
        y: i32,
        rows: i32,
    },
    Leave,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Key(KeyInput),
    /// A key came back up. A menu bar activates on Alt's release,
    /// not on the key after it.
    KeyUp(KeyInput),
    Pointer(PointerInput),
    Resized,
    /// The main window is on an output this big, in logical px: sent
    /// each time it enters one (with several, the last it entered).
    Output((i32, i32)),
    /// The caret's blink flipped (`ctx.caret_on`). It goes to the window
    /// with the keyboard, else the one under the pointer, else nowhere.
    Blink,
    Tick,
    Pasted(String),
    /// The primary selection's text, asked for with `Reply::paste_primary`.
    PastedPrimary(String),
    PastedFiles(FileClip),
    Focus(bool),
    /// The compositor took its shortcuts back from the window that held
    /// them ([`App::shortcuts_inhibited`]), on the user's
    /// Super+Shift+Escape. The modifiers held for that chord are no
    /// longer a hot-key field's to record.
    ShortcutsInactive,
    /// The frame's X, or the window menu's Close. On `MAIN` the program
    /// answers `Reply::quit`; on a child it drops that `Child`. On a
    /// flyout, the compositor took it away; the program stops returning
    /// it from `App::flyout`.
    CloseRequest,
    /// Something off the event loop's thread has work for the program (see
    /// `Waker::wake`). Delivered on `MAIN` even when the main window is
    /// hidden, so `win` means nothing.
    Wake,
    /// A drag from any program came over this window, at `(x, y)`. The
    /// program answers this and every `DragMotion` with
    /// [`Reply::drag_accept`]; without an answer the last one stands, and
    /// a new drag starts refused. `source` is what the dragging program
    /// allows; `action` is what a drop would do as the compositor has it
    /// now (None before the first answer). `own` is a drag this program
    /// started and has not dropped yet, whose files it knows already.
    DragEnter {
        x: i32,
        y: i32,
        offered: Offered,
        source: Actions,
        action: Option<DragAction>,
        own: bool,
    },
    DragMotion {
        x: i32,
        y: i32,
        action: Option<DragAction>,
    },
    /// The files of another program's drag, read ahead of the drop and sent
    /// once after `DragEnter` (an `own` drag's are known), so the program can
    /// pick move or copy, or refuse a folder dropped into itself. Empty for
    /// web addresses.
    DragFiles {
        paths: Vec<PathBuf>,
    },
    DragLeave,
    /// The drag was dropped here; this ends it the way `DragLeave` would.
    /// With `DragAction::Ask` the program asks the user, then answers
    /// with [`Reply::answer_drop`] (now or later), picking among
    /// `source`, what the dragging program allows (a drop menu greys the
    /// rest); any other action is already settled when this arrives.
    DragDrop {
        x: i32,
        y: i32,
        data: Dropped,
        action: DragAction,
        source: Actions,
    },
    /// A drag dropped on a field of the window's `App::drop_form` changed it (`Changed`
    /// with the field's id), as a key or paste in it would; `App::drop_file_dialog`'s File
    /// name reports `fielddrop::FILE_NAME`.
    FieldDropped(reprisekit::controls::form::Event),
    /// The `Ask` drop this window was answering is gone: the dragging
    /// program or the compositor dropped it, or the answer was an action
    /// the source does not allow. Nothing was finished.
    DropCancelled,
    /// The drag this window started with [`Reply::start_drag`] is over.
    /// `dropped`: released over a target that took it. `action`: what the
    /// target said it did, None when it never said (refusal, cancel, or a
    /// target that drops the offer unfinished, as some GTK programs do). The
    /// target does a move itself; the source deletes nothing.
    DragEnd {
        dropped: bool,
        action: Option<DragAction>,
    },
    /// The outputs as the compositor has them, on `MAIN`, after every
    /// change, for a program that asked ([`App::output_management`]).
    /// Empty when the compositor offers no output management.
    Heads(Vec<heads::Head>),
    HeadsApplied {
        serial: u32,
        ok: bool,
    },
}

pub use reprisefs::dropdata::{GNOME_FILES, URI_LIST};
pub use reprisefs::fileclip::FileClip;

/// Run `argv` as a process of its own in working directory `dir` (the caller's when `None`), no
/// shell between, reaped in the background: a [`Reply::spawn`] for a caller that needs the exec's
/// error, as a program file that will not start reports it. With a `scope` the child waits before
/// its exec until it is in that app scope ([`reprisefs::scope::hold`]), so this blocks on the
/// session bus.
pub fn spawn_in(argv: &[OsString], dir: Option<&Path>, scope: Option<&str>) -> std::io::Result<()> {
    spawn_then(argv, dir, scope, |_| {})
}

/// [`spawn_in`]'s spawn, running `then` with the child's pid on the reaper thread before the
/// wait, so the pid cannot be reused under it.
fn spawn_then(
    argv: &[OsString],
    dir: Option<&Path>,
    scope: Option<&str>,
    then: impl FnOnce(u32) + Send + 'static,
) -> std::io::Result<()> {
    let Some((program, args)) = argv.split_first() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "empty command line",
        ));
    };
    let mut c = Command::new(program);
    c.args(args)
        .env_remove(reprisebus::instance::KEYBOARD_LAUNCH_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = dir {
        c.current_dir(dir);
    }
    if let Some(app) = scope {
        reprisefs::scope::hold(&mut c, app)?;
    }
    let mut child = c.spawn()?;
    std::thread::spawn(move || {
        then(child.id());
        let _ = child.wait();
    });
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reply {
    pub redraw: bool,
    pub redraw_all: bool,
    pub quit: bool,
    pub copy: Option<String>,
    /// Arrives as `Event::Pasted` on the window this reply is for.
    pub paste: bool,
    /// Offered as the primary selection, the text a middle click pastes.
    pub copy_primary: Option<String>,
    /// Arrives as `Event::PastedPrimary` on the window this reply is for.
    pub paste_primary: bool,
    /// Run each argument vector as a process of its own, no shell
    /// between, detached and reaped in the background. No activation
    /// token is sent: a launched window takes focus as a child of the
    /// focused caller.
    pub spawn: Vec<Vec<OsString>>,
    /// The desktop entry id `spawn` runs as: each process starts in that app's scope
    /// (`reprisefs::scope`).
    pub spawn_app: Option<String>,
    pub copy_files: Option<FileClip>,
    /// Arrives as `Event::PastedFiles` on the window this reply is for.
    pub paste_files: bool,
    /// What another process handed over with a request to show a window, and
    /// that window: its token is presented for it, so the compositor focuses
    /// it, and a start by key brings out its cues. Dropped while that window
    /// is not shown.
    pub activate: Option<(WinId, reprisebus::instance::Handoff)>,
    /// Start a drag of files from the window this reply is for. It must
    /// answer a pointer press still held (the host uses that press's
    /// serial); the compositor refuses it otherwise.
    pub start_drag: Option<Box<DragOut>>,
    /// Ask again for the running drag's image ([`App::drag_image`]) at the scale it shows at,
    /// for art that landed since it was drawn.
    pub redraw_drag: bool,
    /// The answer to a drag over the window: `Some(None)` refuses it,
    /// `Some(Some(a))` takes it as `a`, `None` leaves the last answer.
    pub drag_accept: Option<Option<Accept>>,
    /// The user's choice for an `Ask` drop: `Some(Some(Copy | Move))`
    /// finishes it with that action, `Some(None)` (or `Ask`) cancels it.
    pub answer_drop: Option<Option<DragAction>>,
    /// Send the window this reply is for one `Event::Tick` in `TICK_MS`,
    /// for something that moves while the user holds still (a drag
    /// resting at an edit's edge scrolls it). A window that wants the
    /// next asks again in its answer to the tick; asking while one is on
    /// its way adds none.
    pub tick: bool,
    /// Resize the window this reply is for from its lower-right corner,
    /// the way its size grip does. It must answer a left press still held
    /// (the host uses that press's serial); the compositor refuses it
    /// otherwise.
    pub resize: bool,
    /// Make the main window this client size (logical px), for a window that
    /// grows with what it shows, held inside the compositor's bounds. A size that
    /// changes the window's sends it `Event::Resized`. It stands until the
    /// compositor asks for another size; a configure that only repeats the last
    /// size asked for leaves it.
    pub size: Option<(i32, i32)>,
    /// More of this program's windows to draw again, besides the one this reply is for.
    pub redraw_wins: Vec<WinId>,
    /// Bring this program's window to the front with the keyboard: the host asks xdg-activation
    /// for a token on the last input's serial and presents it for that window.
    pub raise: Option<WinId>,
}

impl Reply {
    pub fn redraw() -> Reply {
        Reply {
            redraw: true,
            ..Default::default()
        }
    }
    pub fn none() -> Reply {
        Reply::default()
    }
    pub fn redraw_all() -> Reply {
        Reply {
            redraw_all: true,
            ..Default::default()
        }
    }
    pub fn resize() -> Reply {
        Reply {
            resize: true,
            ..Default::default()
        }
    }
    pub fn start_drag(out: DragOut) -> Reply {
        Reply {
            start_drag: Some(Box::new(out)),
            ..Default::default()
        }
    }
    pub fn drag_accept(accept: Option<Accept>) -> Reply {
        Reply {
            drag_accept: Some(accept),
            ..Default::default()
        }
    }
    pub fn answer_drop(action: Option<DragAction>) -> Reply {
        Reply {
            answer_drop: Some(action),
            ..Default::default()
        }
    }
    /// This reply with the clipboard half `clip` read for the same event
    /// ([`keys::form_clipboard`]) added: `clip`'s copy goes first, and
    /// either one asking to paste asks.
    pub fn with_clipboard(self, clip: Reply) -> Reply {
        Reply {
            copy: clip.copy.or(self.copy),
            paste: clip.paste || self.paste,
            ..self
        }
    }

    /// This reply and `then`, the answer of another job landing in the same wake, so neither
    /// loses its effects: flags join, each spawn keeps its own app scope, and for a one-off
    /// (a copy, a drag) this reply's goes first.
    pub fn and(self, then: Reply) -> Reply {
        let Reply {
            redraw,
            redraw_all,
            quit,
            copy,
            paste,
            copy_primary,
            paste_primary,
            spawn,
            spawn_app,
            copy_files,
            paste_files,
            activate,
            start_drag,
            redraw_drag,
            drag_accept,
            answer_drop,
            tick,
            resize,
            size,
            mut redraw_wins,
            raise,
        } = then;
        let (spawn, spawn_app) = if spawn.is_empty() {
            (self.spawn, self.spawn_app)
        } else if self.spawn.is_empty() {
            (spawn, spawn_app)
        } else if self.spawn_app == spawn_app {
            (self.spawn.into_iter().chain(spawn).collect(), spawn_app)
        } else {
            let scoped = |argvs: Vec<Vec<OsString>>, app: Option<String>| {
                argvs
                    .into_iter()
                    .map(move |argv| reprisefs::scope::in_scope(&argv, app.as_deref()))
            };
            let mine = scoped(self.spawn, self.spawn_app);
            (mine.chain(scoped(spawn, spawn_app)).collect(), None)
        };
        Reply {
            redraw: self.redraw || redraw,
            redraw_all: self.redraw_all || redraw_all,
            quit: self.quit || quit,
            copy: self.copy.or(copy),
            paste: self.paste || paste,
            copy_primary: self.copy_primary.or(copy_primary),
            paste_primary: self.paste_primary || paste_primary,
            spawn,
            spawn_app,
            copy_files: self.copy_files.or(copy_files),
            paste_files: self.paste_files || paste_files,
            activate: self.activate.or(activate),
            start_drag: self.start_drag.or(start_drag),
            redraw_drag: self.redraw_drag || redraw_drag,
            drag_accept: self.drag_accept.or(drag_accept),
            answer_drop: self.answer_drop.or(answer_drop),
            tick: self.tick || tick,
            resize: self.resize || resize,
            size: self.size.or(size),
            redraw_wins: {
                let mut wins = self.redraw_wins;
                wins.append(&mut redraw_wins);
                wins
            },
            raise: self.raise.or(raise),
        }
    }
}

pub trait App {
    fn title(&self) -> String;
    fn app_id(&self) -> String;
    fn initial_size(&self) -> (i32, i32);
    /// Whether the main window refuses to resize, like an About box.
    /// Its min and max size are pinned to `initial_size`, and to each later `Reply::size`.
    fn fixed(&self) -> bool {
        false
    }
    fn children(&self) -> Vec<Child> {
        Vec::new()
    }
    /// The window child `win` sits over. `None` makes it a toplevel of its own, unparented like
    /// the main window, for a program that shows several (its `modal` is then ignored).
    fn parent(&self, _win: WinId) -> Option<WinId> {
        Some(MAIN)
    }
    /// The window of this program whose caption tab group unparented child `win` joins as it
    /// maps, right after it. Read when its window is made; needs [`tab_groups`].
    fn tab_with(&self, _win: WinId) -> Option<WinId> {
        None
    }
    /// The smallest client size resizable window `win` can be dragged
    /// to, read when its window is made; `None` keeps a child's wanted
    /// size, or `MIN_SIZE` for the main window, as the floor.
    fn min_size(&self, _win: WinId) -> Option<(i32, i32)> {
        None
    }
    /// The flyout the program has open right now, if any. The host
    /// reconciles it after every event, like `children`.
    fn flyout(&self) -> Option<Flyout> {
        None
    }
    /// Whether the main window has no surface at all (a program that lives
    /// in the tray). The program answers a `CloseRequest` on `MAIN` with
    /// `Reply::redraw()` and returns `true` again, rather than quitting.
    /// While hidden, dialogs are ordinary windows, not children of a parent.
    fn hidden(&self) -> bool {
        false
    }
    /// Another program's window this one's main window belongs to, as
    /// the handle it exported through xdg-foreign (a portal's
    /// `parent_window` without its `wayland:`). The compositor then
    /// centres the main window on that one and keeps it off the taskbar.
    /// A handle the compositor does not know leaves the window ownerless.
    fn foreign_parent(&self) -> Option<String> {
        None
    }
    /// Whether the main window, owned through `foreign_parent`, blocks
    /// its owner until it closes (xdg-dialog-v1 `set_modal`).
    fn modal(&self) -> bool {
        false
    }
    /// The caption `win` wears: a window's by default, a prompt's for a
    /// message box, a help dialog's for a sheet or chooser, a fixed dialog's
    /// for a program that is one dialog. Read after every event; the host
    /// sends it through xdg-toplevel-tag.
    fn caption(&self, _win: WinId) -> Caption {
        Caption::Window
    }
    /// The window that takes the compositor's shortcuts while it has the
    /// keyboard (Super, Alt+Tab and every bind reach it as keys), such as
    /// a dialog whose hot-key field has the focus. Read after every
    /// event; the host holds a keyboard-shortcuts-inhibit on it.
    fn shortcuts_inhibited(&self) -> Option<WinId> {
        None
    }
    /// Called once before the first dispatch, with the handle the
    /// program clones into the threads it spawns.
    fn start(&mut self, _waker: Waker) {}
    /// Whether the program sets the outputs: the host then binds
    /// wlr-output-management, sends `Event::Heads` and carries out
    /// [`Ctx::configure_head`]. Read once, at start.
    fn output_management(&self) -> bool {
        false
    }
    /// Whether [`Ctx::theme`] is at the text size: the program lays its own
    /// lists, trees, menus and bars out with [`Theme::controls`], so they grow
    /// with the text. Read at start and on every look change.
    fn text_sized(&self) -> bool {
        false
    }
    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply;
    /// Called before each draw of `win`, at `scale` device px per logical px, with
    /// [`Ctx::icons`] at that scale: a program fetches again what it holds at another one.
    fn follow_scale(&mut self, _win: WinId, _scale: i32, _ctx: &mut Ctx) {}
    /// The image of the drag window `win` started ([`Reply::start_drag`]), drawn again at
    /// `scale` for an output of another scale, with [`Ctx::icons`] at that scale. None keeps the
    /// image it has.
    fn drag_image(&mut self, _win: WinId, _scale: i32, _ctx: &mut Ctx) -> Option<Canvas> {
        None
    }
    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx);
    /// Whether `draw` gets `win`'s last frame back (until a resize or scale change) and reports
    /// what it redrew with [`Canvas::report_damage`].
    fn keeps_canvas(&self, _win: WinId) -> bool {
        false
    }
    /// Window `win`'s accessibility tree, from the state `draw` last drew:
    /// hang the controls' `access` parts under `reprisekit::access::ROOT` and
    /// set the focus. Called only while a screen reader listens. Without it
    /// the window is an empty, titled one.
    fn access(&mut self, _win: WinId, _tree: &mut Tree, _ctx: &Ctx) {}
    /// The form in window `win` whose single-line edit fields take a drag of text or files
    /// ([`fielddrop`]); its drags then never reach `event`. None (the default) hands them
    /// to `event`.
    fn drop_form(&mut self, _win: WinId) -> Option<DropForm<'_>> {
        None
    }
    /// The form in window `win` that a screen reader's Focus and ScrollIntoView reach without a
    /// click: a row scrolls into view, a button takes the focus unpressed. Defaults to
    /// [`App::drop_form`].
    fn reader_form(&mut self, win: WinId) -> Option<ReaderForm<'_>> {
        self.drop_form(win).map(ReaderForm::Form)
    }
    /// Carry out a screen reader's Focus or ScrollIntoView in window `win` that takes no click;
    /// true when the window changed and is drawn again. Defaults to [`reader_forms`]; a window
    /// drawn from other controls answers it here.
    fn reader(&mut self, win: WinId, act: Reader, ctx: &Ctx) -> bool {
        reader_forms(self, win, act, ctx)
    }
    /// The file dialog in window `win`, whose File name field takes a drag of text or files as
    /// a form's field does; files go in by name, or by path from another folder. None (the
    /// default) hands the drags to `event`. The host also decodes its pictures for Thumbnails
    /// and Tiles and redraws the window as they arrive.
    fn drop_file_dialog(&mut self, _win: WinId) -> Option<FileDrop<'_>> {
        None
    }
}

/// A screen reader's `act` carried out on window `win`'s [`App::reader_form`], else its
/// [`App::drop_file_dialog`]; what [`App::reader`] does unless a program answers it.
pub fn reader_forms<A: App + ?Sized>(app: &mut A, win: WinId, act: Reader, ctx: &Ctx) -> bool {
    let base = ctx.base_px();
    if let Some(f) = app.reader_form(win) {
        return f.act(act, &ctx.text, base);
    }
    let px = ctx.form_units().px(base);
    let Some(FileDrop { dlg, .. }) = app.drop_file_dialog(win) else {
        return false;
    };
    let (w, h) = dlg.client();
    let r = Rect::new(0, 0, w, h);
    let ev = match act {
        Reader::Reveal { at, row } => dlg.reveal(&ctx.text, px, r, at, row),
        Reader::Focus(at) => dlg.focus_at(&ctx.text, px, r, at),
    };
    !matches!(ev, reprisekit::controls::filedlg::Event::None)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Caption {
    #[default]
    Window,
    Prompt,
    Help,
    Fixed,
}

thread_local! {
    /// The host's theme and the main window's output width, for [`caption_fit`].
    static CAPTION_FIT: RefCell<(Option<Theme>, Option<i32>)> = const { RefCell::new((None, None)) };
}

/// The narrowest client width at which a child window's caption shows `title` whole under the
/// host's theme (an owned child wears the dialog caption), no wider than the main window's
/// output allows. 0 before the host has a theme.
pub fn caption_fit(title: &str) -> i32 {
    CAPTION_FIT.with_borrow(|(theme, _)| {
        let Some(theme) = theme else {
            return 0;
        };
        let fit = reprisekit::look::caption_fit(theme, title, reprisekit::frame::Chrome::DIALOG);
        match dialog_max_w() {
            0 => fit,
            max => fit.min(max),
        }
    })
}

/// The widest client width a child dialog's frame leaves on the main window's output, for
/// `msgbox::Fields::max_w`. 0 before the host knows its theme and output.
pub fn dialog_max_w() -> i32 {
    use reprisekit::frame::{Chrome, FrameGeometry};
    CAPTION_FIT.with_borrow(|(theme, output_w)| match (theme, output_w) {
        (Some(theme), Some(w)) => {
            FrameGeometry::client_size_for_total(&theme.metrics, *w, 1, false, Chrome::DIALOG).0
        }
        _ => 0,
    })
}

/// `theme` at `text_size` for an [`App::text_sized`] program, else as it is.
fn sized(theme: Theme, app: &dyn App, text_size: u32) -> Theme {
    if app.text_sized() {
        theme.with_text_size(text_size)
    } else {
        theme
    }
}

fn publish_theme(theme: &Theme) {
    CAPTION_FIT.with_borrow_mut(|(t, _)| *t = Some(theme.clone()));
}

const BLINK_MS: u64 = 530;
pub const TICK_MS: u64 = 50;

/// The largest window side the host draws, in logical px. The shm
/// buffer's `h * w * 4` then stays within `i32` (2^30 at most).
const MAX_SIDE: i32 = 16384;
const _: () = assert!((MAX_SIDE as i64) * (MAX_SIDE as i64) * 4 <= i32::MAX as i64);

/// The smallest a resizable main window may be made, logical px.
pub const MIN_SIZE: (i32, i32) = (120, 60);

/// A window side from the compositor or the program, clamped into
/// `1..=MAX_SIDE` so the canvas and the shm buffer stay allocatable.
fn sane_side(v: i64) -> i32 {
    v.clamp(1, i64::from(MAX_SIDE)) as i32
}

fn sane_size((w, h): (i32, i32)) -> (i32, i32) {
    (sane_side(i64::from(w)), sane_side(i64::from(h)))
}

/// `size` held inside `bounds`, side by side.
fn bounded(size: (i32, i32), bounds: Option<(i32, i32)>) -> (i32, i32) {
    match bounds {
        Some((bw, bh)) => (size.0.min(bw), size.1.min(bh)),
        None => size,
    }
}

/// The size a window of `now` takes for limits `min` and `max`: a fixed window (`max`) its
/// size; a resizable one at its program's pick the size the program picks now (`natural`),
/// else its own size; either no smaller than `min`.
fn regrown(
    now: (i32, i32),
    min: (i32, i32),
    max: Option<(i32, i32)>,
    natural: Option<(i32, i32)>,
) -> (i32, i32) {
    let size = natural.unwrap_or(now);
    max.unwrap_or((size.0.max(min.0), size.1.max(min.1)))
}

/// Whether a window of `size` is at the size its program picked (`picked`), held inside
/// `bounds`: neither the user nor the compositor sized it since.
fn at_pick(size: (i32, i32), picked: (i32, i32), bounds: Option<(i32, i32)>) -> bool {
    size == bounded(picked, bounds)
}

/// The size a window of `size` takes when its bounds go from `was` to `now`: one at its
/// program's pick under `was` stays at the pick, any other keeps its size; either held inside
/// `now`.
fn rebounded(
    size: (i32, i32),
    picked: (i32, i32),
    was: Option<(i32, i32)>,
    now: Option<(i32, i32)>,
) -> (i32, i32) {
    match at_pick(size, picked, was) {
        true => bounded(picked, now),
        false => bounded(size, now),
    }
}

/// The main window's opening client size and its floor: a resizable window
/// opens no smaller than its minimum, whatever size the program kept.
fn opening_size(app: &dyn App) -> ((i32, i32), (i32, i32)) {
    let size = sane_size(app.initial_size());
    if app.fixed() {
        return (size, size);
    }
    let min = app.min_size(MAIN).map_or(MIN_SIZE, sane_size);
    ((size.0.max(min.0), size.1.max(min.1)), min)
}

/// The size a configure leaves a window at, and whether the program
/// still owns it. `asked` is what the configure asks per side (`None`
/// leaves that side to the window), `last` what the one before asked.
/// A size the program picked (`own`) stands against a configure that
/// repeats `last` outside a resize drag: that one is about the window's
/// state, and the compositor still has the old size in it. Any other
/// size asked for is the compositor's, or the user's, and wins.
fn configured_size(
    now: (i32, i32),
    asked: (Option<i32>, Option<i32>),
    last: (Option<i32>, Option<i32>),
    own: bool,
    resizing: bool,
) -> ((i32, i32), bool) {
    if own && asked == last && !resizing {
        return (now, true);
    }
    ((asked.0.unwrap_or(now.0), asked.1.unwrap_or(now.1)), false)
}

/// The size a configure leaves a window at, held inside `bounds` on each side the window
/// picks itself (`asked` leaves it); a side the compositor asks for is its own.
fn held_size(
    size: (i32, i32),
    asked: (Option<i32>, Option<i32>),
    bounds: Option<(i32, i32)>,
) -> (i32, i32) {
    let held = bounded(size, bounds);
    (
        asked.0.map_or(held.0, |_| size.0),
        asked.1.map_or(held.1, |_| size.1),
    )
}

/// The theme a program builds from the compositor's environment: the
/// interface named by [`reprisekit::interface::ENV`] (Blue variant from
/// [`reprisekit::blue::ENV`], Glass mode and material from
/// [`reprisekit::glass::MODE_ENV`] and [`reprisekit::glass::MATERIAL_ENV`]), else
/// [`Theme::default`]; then a
/// [`reprisekit::theme::COLORS_ENV`] scheme is laid over it.
fn theme_from_env(
    interface: Option<&str>,
    variant: Option<&str>,
    glass_mode: Option<&str>,
    glass_material: Option<&str>,
    colors: Option<&str>,
) -> Theme {
    let mut theme = reprisekit::interfaces::theme_with(
        reprisekit::interface::Interface::from_env_value(interface),
        reprisekit::blue::Variant::from_env_value(variant),
        &reprisekit::glass::Settings {
            mode: glass_mode
                .and_then(reprisekit::glass::Mode::parse)
                .unwrap_or_default(),
            material: glass_material
                .and_then(reprisekit::glass::Material::parse)
                .unwrap_or_default(),
            ..reprisekit::glass::Settings::default()
        },
    );
    if let Some(s) = colors
        && let Some(cs) = reprisekit::theme::ColorScheme::from_env_over(theme.colors.clone(), s)
    {
        theme.colors = cs;
    }
    theme
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Look {
    interface: Option<String>,
    blue_variant: Option<String>,
    glass_mode: Option<String>,
    glass_material: Option<String>,
    colors: Option<String>,
    icon_theme: String,
    /// `[fonts] shell_dlg`, read from `config.toml` at the start and
    /// followed through the portal after: the second dialog font every
    /// renderer loads beside the UI font.
    shell_dlg: Option<String>,
    /// `[fonts] family`, read and followed as `shell_dlg` is; `None` keeps the interface's own.
    font_family: Option<Vec<String>>,
    /// `[fonts] antialias`, read and followed as `shell_dlg` is; `None` keeps the interface's own.
    antialias: Option<bool>,
    /// `[accessibility] text_size` in percent, read and followed as
    /// `shell_dlg` is, for [`Ctx::text_size`].
    text_size: u32,
    /// `[metrics] double_click_ms`, read and followed as `shell_dlg` is; `None` keeps the
    /// interface's own.
    double_click_ms: Option<i32>,
}

impl Look {
    fn from_env() -> Look {
        Look {
            interface: std::env::var(reprisekit::interface::ENV).ok(),
            blue_variant: std::env::var(reprisekit::blue::ENV).ok(),
            glass_mode: std::env::var(reprisekit::glass::MODE_ENV).ok(),
            glass_material: std::env::var(reprisekit::glass::MATERIAL_ENV).ok(),
            colors: std::env::var(reprisekit::theme::COLORS_ENV).ok(),
            icon_theme: std::env::var(reprisekit::icontheme::ENV)
                .unwrap_or_else(|_| "hicolor".into()),
            shell_dlg: repriseconf::shell_dlg(),
            font_family: repriseconf::font_family(),
            antialias: repriseconf::font_antialias(),
            text_size: repriseconf::text_size(),
            double_click_ms: repriseconf::double_click_ms(),
        }
    }

    fn theme(&self) -> Theme {
        let mut theme = theme_from_env(
            self.interface.as_deref(),
            self.blue_variant.as_deref(),
            self.glass_mode.as_deref(),
            self.glass_material.as_deref(),
            self.colors.as_deref(),
        );
        theme.fonts.shell_dlg = self.shell_dlg.clone();
        if let Some(family) = &self.font_family {
            theme.fonts.family = family.clone();
        }
        if let Some(antialias) = self.antialias {
            theme.fonts.antialias = antialias;
        }
        if let Some(ms) = self.double_click_ms {
            theme.metrics.double_click_ms = ms;
        }
        theme
    }

    fn icons(&self) -> IconCache {
        IconCache::new(IconTheme::load(
            &self.icon_theme,
            &IconTheme::default_roots(),
        ))
    }

    fn set(&mut self, key: &str, value: String) -> bool {
        use reprisebus::appearance as a;
        if key == a::SHELL_DLG {
            let font = Some(value).filter(|s| !s.trim().is_empty());
            let changed = self.shell_dlg != font;
            self.shell_dlg = font;
            return changed;
        }
        if key == a::FONT_FAMILY {
            let family = Some(a::families(&value)).filter(|f| !f.is_empty());
            let changed = self.font_family != family;
            self.font_family = family;
            return changed;
        }
        if key == a::FONT_ANTIALIAS {
            let Ok(antialias) = value.parse() else {
                return false;
            };
            let changed = self.antialias != Some(antialias);
            self.antialias = Some(antialias);
            return changed;
        }
        if key == a::DOUBLE_CLICK_MS {
            let Ok(ms) = value.parse() else {
                return false;
            };
            let changed = self.double_click_ms != Some(ms);
            self.double_click_ms = Some(ms);
            return changed;
        }
        if key == a::TEXT_SIZE {
            let Ok(size) = value.parse() else {
                return false;
            };
            let changed = self.text_size != size;
            self.text_size = size;
            return changed;
        }
        let field = match key {
            a::INTERFACE => self.interface.get_or_insert_default(),
            a::BLUE_VARIANT => self.blue_variant.get_or_insert_default(),
            a::GLASS_MODE => self.glass_mode.get_or_insert_default(),
            a::GLASS_MATERIAL => self.glass_material.get_or_insert_default(),
            a::COLORS => self.colors.get_or_insert_default(),
            a::ICON_THEME => &mut self.icon_theme,
            _ => return false,
        };
        let changed = *field != value;
        *field = value;
        changed
    }
}

const TEXT_MIME: &str = "text/plain;charset=utf-8";

/// The theme `run` starts in, which a dialog built before `run` sizes itself for.
pub fn start_theme() -> Theme {
    Look::from_env().theme()
}

thread_local! {
    static TEXT: RefCell<Option<(reprisekit::theme::FontConfig, Rc<TextRenderer>)>> =
        const { RefCell::new(None) };
}

/// A renderer for `fonts`, the one loaded last when the fonts are the same.
fn text_for(fonts: &reprisekit::theme::FontConfig) -> Rc<TextRenderer> {
    TEXT.with_borrow_mut(|held| match held {
        Some((f, text)) if f == fonts => text.clone(),
        _ => {
            let text = Rc::new(TextRenderer::load(fonts));
            *held = Some((fonts.clone(), text.clone()));
            text
        }
    })
}

/// [`Ctx::text`] as `run` starts, for a renderer a program needs before its first event.
pub fn start_text() -> Rc<TextRenderer> {
    text_for(&start_theme().fonts)
}

/// [`Ctx::forms`] as the program starts, for an [`App::initial_size`] asked before any event.
pub fn start_forms() -> Theme {
    let look = Look::from_env();
    look.theme().with_form_text(look.text_size)
}

pub fn run(app: Box<dyn App>) {
    filedlg::remember_views();
    let conn = Connection::connect_to_env().expect("WAYLAND_DISPLAY");
    hangup::on_signals(conn.backend().poll_fd());
    let (globals, event_queue) = registry_queue_init(&conn).expect("registry");
    let qh = event_queue.handle();
    let mut event_queue = event_queue;
    let mut event_loop: EventLoop<Host> = EventLoop::try_new().expect("event loop");
    let loop_handle = event_loop.handle();

    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor");
    let xdg_shell = XdgShell::bind(&globals, &qh).expect("xdg_wm_base");
    let layer_shell = LayerShell::bind(&globals, &qh).ok();
    let activation = ActivationState::bind(&globals, &qh).ok();
    let shm = Shm::bind(&globals, &qh).expect("wl_shm");
    let data_device_manager = DataDeviceManagerState::bind(&globals, &qh).ok();
    let viewporter = globals
        .bind::<wp_viewporter::WpViewporter, _, _>(&qh, 1..=1, NoEvents)
        .ok();
    let fractional = globals
        .bind::<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, _, _>(
            &qh,
            1..=1,
            NoEvents,
        )
        .ok();
    let text_input_manager = globals
        .bind::<zwp_text_input_manager_v3::ZwpTextInputManagerV3, _, _>(&qh, 1..=1, NoEvents)
        .ok();
    let importer_v2 = globals
        .bind::<zxdg_importer_v2::ZxdgImporterV2, _, _>(&qh, 1..=1, NoEvents)
        .ok();
    let importer_v1 = globals
        .bind::<zxdg_importer_v1::ZxdgImporterV1, _, _>(&qh, 1..=1, NoEvents)
        .ok();
    let wm_dialog = globals
        .bind::<xdg_wm_dialog_v1::XdgWmDialogV1, _, _>(&qh, 1..=1, NoEvents)
        .ok();
    let inhibit_manager = globals
        .bind::<zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1, _, _>(
            &qh,
            1..=1,
            NoEvents,
        )
        .ok();
    let tag_manager = globals
        .bind::<xdg_toplevel_tag_manager_v1::XdgToplevelTagManagerV1, _, _>(&qh, 1..=1, NoEvents)
        .ok();
    let effects = globals
        .bind::<ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1, _, _>(
            &qh,
            1..=1,
            NoEvents,
        )
        .ok();
    let tab_groups = globals
        .bind::<reprise_tabs_v1::RepriseTabsV1, _, _>(&qh, 1..=1, NoEvents)
        .ok();
    TAB_GROUPS.store(tab_groups.is_some(), std::sync::atomic::Ordering::Relaxed);
    // The main window itself is `sync_main`'s, so a program that starts
    // hidden never gets a surface for it.
    let ((w, h), _) = opening_size(&*app);
    let look = Look::from_env();
    let theme = look.theme();
    publish_theme(&theme);
    let text = text_for(&theme.fonts);
    let theme = sized(theme, &*app, look.text_size);
    let icons = look.icons();
    let (ping, ping_source) = make_ping().expect("ping");
    let (a11y, a11y_source) = channel();
    // True once the accessibility bus answers; false when it came back after a restart.
    let (a11y_ready, a11y_ready_source) = channel::<bool>();
    let (look_tx, look_source) = channel::<(String, String)>();
    let (portal_done, portal_source) = channel::<OnLoop>();
    let mut host = Host {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        data_device_manager,
        data_device: None,
        copy_source: None,
        primary: primary::Primary::bind(&globals, &qh),
        copy_text: Arc::default(),
        copy_clip: None,
        drag_out: None,
        drag_in: None,
        field_drag: None,
        edit_menu: Default::default(),
        menu_popups: None,
        asked_drop: None,
        ticking: Vec::new(),
        portal_done,
        press_serial: 0,
        input_serial: 0,
        kb_on: None,
        kb_settle: false,
        key_last: reprisebus::instance::Handoff::from_env().by_key,
        press_time: 0,
        compositor,
        xdg_shell,
        layer_shell,
        activation,
        viewporter,
        fractional,
        text_input_manager,
        text_input: None,
        importer_v2,
        importer_v1,
        wm_dialog,
        tag_manager,
        effects,
        tab_groups,
        outputs: None,
        inhibit_manager,
        seat: None,
        inhibitor: None,
        foreign: None,
        output_scale: 1,
        slots: Vec::new(),
        focus: None,
        keyboard: None,
        pointer: None,
        mods: Modifiers::default(),
        pointer_pos: (0, 0),
        pointer_in: None,
        loop_handle: loop_handle.clone(),
        qh: qh.clone(),
        app,
        ctx: Ctx {
            theme,
            text_size: look.text_size,
            text,
            icons,
            caret_on: true,
            size: (w, h),
            maximized: false,
            origin: (0, 0),
            wake: Waker(Arc::new(ping)),
            heads: heads::HeadRequests::default(),
            want_tick: Default::default(),
            at_pick: false,
        },
        exit: false,
        last_serial: 0,
        a11y,
        a11y_bus: false,
        key_echo: None,
        look,
    };
    // Learn the outputs' scales before the program loads an icon or opens
    // a window; a window starts at the largest.
    if let Err(e) = event_queue.roundtrip(&mut host) {
        log::warn!("wayland roundtrip for the outputs: {e}");
    }
    host.output_scale = host
        .output_state
        .outputs()
        .filter_map(|o| host.output_state.info(&o).map(|i| i.scale_factor))
        .max()
        .unwrap_or(1)
        .max(1);
    host.ctx.icons.set_scale(host.output_scale);
    // Only a program that sets the outputs binds it, after the roundtrip
    // above: its events wait for the loop, so the first `Event::Heads`
    // follows `App::start` and the program's windows know their scale.
    let wants_outputs = host.app.output_management();
    if wants_outputs {
        host.outputs = outputmgmt::bind(&globals, &host.qh);
    }
    let no_outputs = wants_outputs && host.outputs.is_none();
    WaylandSource::new(conn.clone(), event_queue)
        .insert(loop_handle.clone())
        .expect("wayland source");
    loop_handle
        .insert_source(ping_source, |(), &mut (), host: &mut Host| {
            host.send(MAIN, Event::Wake);
            let wins: Vec<WinId> = host.slots.iter().map(|s| s.id).collect();
            for win in filedlg::take_thumbs(&mut *host.app, wins, &mut host.ctx.icons) {
                host.mark_dirty(win);
            }
        })
        .expect("ping source");
    loop_handle
        .insert_source(a11y_source, |msg, _, host: &mut Host| {
            if let reprise_channel::Event::Msg(msg) = msg {
                host.access_msg(msg);
            }
        })
        .expect("access source");
    loop_handle
        .insert_source(a11y_ready_source, |msg, _, host: &mut Host| {
            if let reprise_channel::Event::Msg(first) = msg {
                host.a11y_bus = true;
                for s in &mut host.slots {
                    if !first {
                        s.access = access::SlotAccess::new(s.id, None);
                    }
                    s.access.connect(s.id, &host.a11y, host.focus == Some(s.id));
                }
            }
        })
        .expect("access ready source");
    reprisebus::a11y::when_bus_ready(move || {
        let _ = a11y_ready.send(true);
        reprisebus::a11y::on_bus_restart(move || {
            let _ = a11y_ready.send(false);
        });
    });
    loop_handle
        .insert_source(look_source, |msg, _, host: &mut Host| {
            if let reprise_channel::Event::Msg((key, value)) = msg {
                host.relook(&key, value);
            }
        })
        .expect("look source");
    loop_handle
        .insert_source(portal_source, |msg, _, host: &mut Host| {
            if let reprise_channel::Event::Msg(done) = msg {
                done(host);
            }
        })
        .expect("portal source");
    reprisebus::filetransfer::connect();
    reprisebus::appearance::watch(move |key, value| {
        let _ = look_tx.send((key.to_string(), value));
    });
    let waker = host.ctx.wake.clone();
    host.app.start(waker);
    if no_outputs {
        host.send(MAIN, Event::Heads(Vec::new()));
    }
    host.sync_main();
    // A program may start with its flyout up; no event would reconcile it otherwise.
    host.sync_flyout();
    loop_handle
        .insert_source(
            Timer::from_duration(Duration::from_millis(BLINK_MS)),
            |_, _, host: &mut Host| {
                host.ctx.caret_on = !host.ctx.caret_on;
                if let Some(win) = blink_target(host.focus, host.pointer_in)
                    && host.slots.iter().any(|s| s.id == win)
                {
                    host.send(win, Event::Blink);
                }
                TimeoutAction::ToDuration(Duration::from_millis(BLINK_MS))
            },
        )
        .expect("timer");

    loop {
        // Everything that dirties a slot is a source here, so nothing needs polling.
        if let Err(e) = turn(&mut event_loop, &mut host, &conn) {
            log::error!("{e}; exiting");
            std::process::exit(1);
        }
        let ready: Vec<WinId> = host
            .slots
            .iter()
            .filter(|s| s.dirty && s.configured)
            .map(|s| s.id)
            .collect();
        for id in ready {
            if let Some(s) = host.slot_mut(id) {
                s.dirty = false;
            }
            host.draw(id, &qh);
        }
        // After the draws: a window's draw opens the menu a right press asked for.
        host.settle_keyboard();
        host.sync_menu();
        if host.exit {
            let _ = conn.flush();
            hangup::hang_up(conn.backend().poll_fd());
            break;
        }
    }
}

/// Runs one turn of a loop holding a `WaylandSource`. The source logs and drops a
/// protocol error, and the closed socket then stays readable, so the stored error ends the loop here.
fn turn<D>(
    event_loop: &mut EventLoop<'_, D>,
    data: &mut D,
    conn: &Connection,
) -> Result<(), String> {
    event_loop
        .dispatch(None, data)
        .map_err(|e| format!("wayland connection lost: {e}"))?;
    match conn.protocol_error() {
        Some(e) => Err(format!("wayland protocol error: {e}")),
        None => Ok(()),
    }
}

///
/// Field order is drop order: the surface (`window`) goes before the
/// `wl_buffer`s it still has attached, and `pool` after both. The
/// viewport and fractional-scale objects are destroyed before any field.
struct Slot {
    id: WinId,
    window: Shell,
    scaling: Scaling,
    /// Drawn into in turn: the compositor keeps the attached one until the next commit.
    buffers: [Option<Buffer>; 2],
    attached: usize,
    canvas: Canvas,
    /// One pool per slot, sized for one buffer (grown for the second) and
    /// replaced on resize: a shared pool doubles on every busy-buffer miss
    /// during a resize until `wl_shm_pool.resize` overflows.
    pool: Option<SlotPool>,
    title: String,
    size: (i32, i32),
    origin: (i32, i32),
    maximized: bool,
    asked: (Option<i32>, Option<i32>),
    own: bool,
    /// The largest client size the compositor suggests (xdg-toplevel `configure_bounds`): its
    /// work area less the frame. The window, its own sizes and its minimum stay inside it.
    bounds: Option<(i32, i32)>,
    /// The size the program last asked for a child window ([`Child::size`]).
    wanted: (i32, i32),
    /// The size the program last gave the window: its opening size, a `Reply::size` or a refit.
    picked: (i32, i32),
    /// The compositor has the toplevel activated; a flyout goes by the
    /// keyboard. A new toplevel counts as active until a configure after its
    /// first buffer says otherwise: focus is decided after that buffer.
    activated: bool,
    configured: bool,
    dirty: bool,
    /// The last buffer is not shown yet; `draw` waits for its frame
    /// callback so a lagging compositor cannot queue a pool per configure
    /// (EMFILE).
    frame_pending: bool,
    mapped: bool,
    modal: bool,
    caption: Caption,
    access: access::SlotAccess,
    /// The window's blur region object and the glass rows across its width it last set.
    glass: Option<ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1>,
    glass_top: (i32, i32),
    /// The window's keyboard cues; `keys` is drawn as `Theme::access_keys`.
    cues: Cues,
    /// The child's [`App::parent`] as the host last set it; none for the main window.
    parent: Option<WinId>,
    /// The focused field the last frame drew.
    spot: Option<TextSpot>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(v) = self.scaling.viewport.take() {
            v.destroy();
        }
        if let Some(f) = self.scaling.fraction.take() {
            f.destroy();
        }
        if let Some(g) = self.glass.take() {
            g.destroy();
        }
    }
}

struct Scaling {
    /// The last `wp_fractional_scale_v1.preferred_scale`, in 120ths.
    fraction_120: Option<u32>,
    /// The outputs' largest scale until a `preferred_buffer_scale` arrives.
    buffer_scale: i32,
    drawn: i32,
    /// With a viewport the buffer is drawn at the draw scale and shown at
    /// the logical size; without one `wl_surface.set_buffer_scale` says it.
    viewport: Option<wp_viewport::WpViewport>,
    fraction: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
}

/// The integer scale a window of logical size `(w, h)` is drawn at: the
/// fractional scale rounded up when the compositor sends one, else its
/// buffer scale, lowered until neither side of the buffer passes
/// `MAX_SIDE`.
fn draw_scale(fraction_120: Option<u32>, buffer_scale: i32, (w, h): (i32, i32)) -> i32 {
    let want = fraction_120.map_or(buffer_scale, |n| n.div_ceil(120) as i32);
    want.min(MAX_SIDE / w.max(h).max(1)).max(1)
}

enum Shell {
    Toplevel(Window),
    Layer(LayerSurface),
}

impl Shell {
    fn wl_surface(&self) -> &wl_surface::WlSurface {
        match self {
            Shell::Toplevel(w) => w.wl_surface(),
            Shell::Layer(l) => l.wl_surface(),
        }
    }

    fn commit(&self) {
        self.wl_surface().commit();
    }

    fn toplevel(&self) -> Option<&Window> {
        match self {
            Shell::Toplevel(w) => Some(w),
            Shell::Layer(_) => None,
        }
    }
}

struct Host {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    data_device_manager: Option<DataDeviceManagerState>,
    data_device: Option<DataDevice>,
    copy_source: Option<CopyPasteSource>,
    primary: primary::Primary,
    copy_text: Arc<[u8]>,
    /// The copied files, as each type they are offered as.
    copy_clip: Option<Vec<(&'static str, Vec<u8>)>>,
    drag_out: Option<Outgoing>,
    drag_in: Option<Incoming>,
    /// A drag over a window's `App::drop_form`.
    field_drag: Option<(WinId, fielddrop::Hover)>,
    edit_menu: editmenu::EditMenu,
    /// The edit menu's popups while it is open.
    menu_popups: Option<menupopup::MenuPopups>,
    asked_drop: Option<Asked>,
    ticking: Vec<WinId>,
    /// Where a document portal call's thread sends what to do with its
    /// answer (`reprisebus::filetransfer`).
    portal_done: Sender<OnLoop>,
    /// The last pointer press's serial, which a drag must name.
    press_serial: u32,
    /// The last key or pointer press's serial, which a menu's grab names.
    input_serial: u32,
    /// The surface the keyboard is on and the window it counts for: a window's menu popups
    /// count for the window.
    kb_on: Option<(wl_surface::WlSurface, WinId)>,
    /// The keyboard left a surface of a window whose menu is open: `settle_keyboard` decides
    /// after the turn whether it went to the menu or away.
    kb_settle: bool,
    /// The last input was a key, not a pointer press, counting a start from the keyboard: a window
    /// opened now underlines its access keys.
    key_last: bool,
    /// The time the last pointer press carried, real or a reader's
    /// (`access_msg`): a reader's click is stamped a second past the
    /// double-click time after it, so two never make a double click.
    press_time: u32,
    compositor: CompositorState,
    xdg_shell: XdgShell,
    layer_shell: Option<LayerShell>,
    activation: Option<ActivationState>,
    viewporter: Option<wp_viewporter::WpViewporter>,
    fractional: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    text_input_manager: Option<zwp_text_input_manager_v3::ZwpTextInputManagerV3>,
    text_input: Option<textinput::TextInput>,
    importer_v2: Option<zxdg_importer_v2::ZxdgImporterV2>,
    importer_v1: Option<zxdg_importer_v1::ZxdgImporterV1>,
    tag_manager: Option<xdg_toplevel_tag_manager_v1::XdgToplevelTagManagerV1>,
    effects: Option<ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1>,
    tab_groups: Option<reprise_tabs_v1::RepriseTabsV1>,
    outputs: Option<outputmgmt::Outputs>,
    inhibit_manager:
        Option<zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1>,
    /// The seat the keyboard came from, which an inhibitor and a resize
    /// from a size grip name.
    seat: Option<wl_seat::WlSeat>,
    /// The shortcuts inhibitor `App::shortcuts_inhibited` asked for, and
    /// the window it is on.
    inhibitor: Option<(
        WinId,
        zwp_keyboard_shortcuts_inhibitor_v1::ZwpKeyboardShortcutsInhibitorV1,
    )>,
    wm_dialog: Option<xdg_wm_dialog_v1::XdgWmDialogV1>,
    /// The main window's foreign parent, alive as long as the window:
    /// set by `make_main`, dropped when `sync_main` hides it.
    foreign: Option<Foreign>,
    output_scale: i32,
    /// `MAIN` is first whenever it exists; a hidden main window has no slot.
    slots: Vec<Slot>,
    focus: Option<WinId>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    mods: Modifiers,
    pointer_pos: (i32, i32),
    pointer_in: Option<WinId>,
    loop_handle: LoopHandle<'static, Host>,
    qh: QueueHandle<Host>,
    app: Box<dyn App>,
    ctx: Ctx,
    exit: bool,
    last_serial: u32,
    a11y: Sender<access::Msg>,
    /// The accessibility bus answered: windows get adapters from then on.
    a11y_bus: bool,
    key_echo: Option<reprisebus::a11y::KeyEcho>,
    look: Look,
}

impl Host {
    fn slot_of(&self, surface: &wl_surface::WlSurface) -> Option<WinId> {
        self.slots
            .iter()
            .find(|s| s.window.wl_surface() == surface)
            .map(|s| s.id)
    }

    fn slot_mut(&mut self, id: WinId) -> Option<&mut Slot> {
        self.slots.iter_mut().find(|s| s.id == id)
    }

    fn scaling(&self, surface: &wl_surface::WlSurface) -> Scaling {
        Scaling {
            fraction_120: None,
            buffer_scale: self.output_scale,
            drawn: 0,
            viewport: self
                .viewporter
                .as_ref()
                .map(|v| v.get_viewport(surface, &self.qh, NoEvents)),
            fraction: self
                .fractional
                .as_ref()
                .map(|f| f.get_fractional_scale(surface, &self.qh, SurfaceOf(surface.clone()))),
        }
    }

    fn rescale(
        &mut self,
        surface: &wl_surface::WlSurface,
        fraction_120: Option<u32>,
        buffer_scale: Option<i32>,
    ) {
        let Some(id) = self.slot_of(surface) else {
            return;
        };
        let Some(s) = self.slot_mut(id) else {
            return;
        };
        if let Some(n) = fraction_120 {
            s.scaling.fraction_120 = Some(n);
        }
        if let Some(n) = buffer_scale {
            s.scaling.buffer_scale = n.max(1);
        }
        if draw_scale(s.scaling.fraction_120, s.scaling.buffer_scale, s.size) != s.scaling.drawn {
            s.dirty = true;
        }
    }

    /// The modal child gating `win`: the newest over `win`'s own window.
    fn modal(&self, win: WinId) -> Option<WinId> {
        let slots = self.slots.iter().map(|s| (s.id, s.modal));
        modal_over(win, slots, |w| self.app.parent(w))
    }

    fn gated(&self, win: WinId, ev: &Event) -> bool {
        let flyout = self
            .slots
            .iter()
            .any(|s| s.id == win && s.window.toplevel().is_none());
        input_blocked(ev, self.modal(win), win, flyout)
    }

    fn send(&mut self, win: WinId, ev: Event) {
        if self.gated(win, &ev) {
            return;
        }
        match self.slots.iter().find(|s| s.id == win) {
            Some(s) => self.ctx.aim(s),
            // A hidden main window still takes events the host has no
            // surface for (a `Wake`, a blink); a child with no surface
            // does not exist at all.
            None if win == MAIN => self.ctx.origin = (0, 0),
            None => return,
        }
        // Anything but the blink may change what a screen reader is
        // told, in any window (a dialog's OK retitles the main one).
        if ev != Event::Blink {
            for s in &mut self.slots {
                s.access.state.dirty = true;
            }
        }
        if self.edit_menu_event(win, &ev) {
            return;
        }
        if self.file_dialog_clipboard(win, &ev) {
            return;
        }
        if let Some((reply, changed)) = self.field_drop(win, &ev) {
            self.apply(win, reply);
            if changed != reprisekit::controls::form::Event::None {
                self.send(win, Event::FieldDropped(changed));
            }
            return;
        }
        filedlg::thumbs_on(&mut *self.app, win, &self.ctx.wake);
        // Read off the live offer: sctk calls `selection` for a new offer but not for a cleared one.
        let offer = self.selection_offer();
        CLIPBOARD_TEXT.set(
            offer
                .as_ref()
                .is_some_and(|o| o.with_mime_types(text_mime).is_some()),
        );
        CLIPBOARD_FILES
            .set(offer.is_some_and(|o| o.with_mime_types(reprisefs::fileclip::source).is_some()));
        let mut reply = self
            .app
            .event(win, ev, &mut self.ctx)
            .with_clipboard(keys::take_clipboard());
        reply.tick |= self.ctx.want_tick.take();
        self.apply(win, reply);
    }

    /// Whether the clipboard holds text, read off the live offer as `CLIPBOARD_TEXT` is.
    fn clipboard_text(&self) -> bool {
        self.selection_offer()
            .is_some_and(|o| o.with_mime_types(text_mime).is_some())
    }

    /// The edit menu's share of `ev` ([`editmenu`]); `true` when the program does not get it.
    /// The menu shows on popups ([`menupopup`]), so only a pending right press draws the window
    /// again. A window keeping its canvas has none.
    fn edit_menu_event(&mut self, win: WinId, ev: &Event) -> bool {
        let Some(i) = self.slots.iter().position(|s| s.id == win) else {
            return false;
        };
        if self.app.keeps_canvas(win) {
            return false;
        }
        let within = self.menu_within(self.slots[i].size);
        let cx = MenuCtx {
            text: &self.ctx.text,
            theme: &self.ctx.theme,
            px: self.ctx.theme.fonts.size_px,
            within,
        };
        let paste = self.clipboard_text();
        let out = self
            .edit_menu
            .event(win, ev, self.slots[i].spot.as_ref(), paste, &cx);
        self.slots[i].dirty |= self.edit_menu.is_pending(win);
        match out {
            editmenu::Outcome::Pass => false,
            editmenu::Outcome::Taken | editmenu::Outcome::Redraw => true,
            editmenu::Outcome::Run(k) => {
                self.send(win, Event::Key(k));
                true
            }
        }
    }

    /// What a menu fits its width to: the largest output, as a popup may run past the window.
    fn menu_within(&self, size: (i32, i32)) -> Rect {
        let (w, h) = self
            .output_state
            .outputs()
            .filter_map(|o| self.output_state.info(&o)?.logical_size)
            .max_by_key(|&(w, h)| i64::from(w) * i64::from(h))
            .unwrap_or(size);
        Rect::new(0, 0, w, h)
    }

    /// The clipboard for a file dialog's File name ([`App::drop_file_dialog`]): Ctrl+C and Ctrl+X
    /// put its selection on the clipboard before the key reaches the dialog, and Ctrl+V asks for
    /// the clipboard's text, which goes into the field. `true` when the program does not get `ev`.
    fn file_dialog_clipboard(&mut self, win: WinId, ev: &Event) -> bool {
        let Some(fd) = self.app.drop_file_dialog(win) else {
            return false;
        };
        match ev {
            Event::Key(KeyInput {
                edit: Some(EditKey::Copy | EditKey::Cut),
                ..
            }) => {
                let text = (fd.dlg.focused_edit())
                    .filter(|e| !e.is_secret())
                    .map(|e| e.selected_text().to_string())
                    .filter(|t| !t.is_empty());
                if let Some(t) = text {
                    self.copy(t);
                }
                false
            }
            Event::Key(KeyInput {
                edit: Some(EditKey::Paste),
                ..
            }) if fd.dlg.focused_edit().is_some() => {
                self.apply(
                    win,
                    Reply {
                        paste: true,
                        ..Reply::none()
                    },
                );
                true
            }
            Event::Pasted(text) => {
                if fd.dlg.paste(text)
                    && let Some(s) = self.slot_mut(win)
                {
                    s.dirty = true;
                }
                true
            }
            _ => false,
        }
    }

    /// `ev` for window `win`'s drop form, when it has one and `ev` is a drag over it or a tick
    /// while one rests on it.
    fn field_drop(
        &mut self,
        win: WinId,
        ev: &Event,
    ) -> Option<(Reply, reprisekit::controls::form::Event)> {
        let drag = matches!(
            ev,
            Event::DragEnter { .. }
                | Event::DragMotion { .. }
                | Event::DragFiles { .. }
                | Event::DragLeave
                | Event::DragDrop { .. }
        );
        let mut hover = self.field_drag.filter(|(w, _)| *w == win).map(|(_, h)| h);
        if !drag && !(hover.is_some() && *ev == Event::Tick) {
            return None;
        }
        let base = self.ctx.base_px();
        let out = if let Some(DropForm { form, px }) = self.app.drop_form(win) {
            let px = px.unwrap_or_else(|| form.units().px(base));
            fielddrop::event(form, &mut hover, ev, &self.ctx.text, px)
        } else {
            let px = self.ctx.form_units().px(base);
            let mut file = self.app.drop_file_dialog(win)?;
            fielddrop::event(&mut file, &mut hover, ev, &self.ctx.text, px)
        };
        self.field_drag = hover.map(|h| (win, h));
        out
    }

    fn arm_tick(&mut self, win: WinId) {
        if self.ticking.contains(&win) {
            return;
        }
        self.ticking.push(win);
        let _ = self.loop_handle.insert_source(
            Timer::from_duration(Duration::from_millis(TICK_MS)),
            move |_, _, host: &mut Host| {
                host.ticking.retain(|w| *w != win);
                if host.slots.iter().any(|s| s.id == win) {
                    host.send(win, Event::Tick);
                }
                TimeoutAction::Drop
            },
        );
    }

    fn apply(&mut self, win: WinId, reply: Reply) {
        if reply.redraw
            && let Some(s) = self.slot_mut(win)
        {
            s.dirty = true;
        }
        if reply.redraw_all {
            for s in self.slots.iter_mut() {
                s.dirty = true;
            }
        }
        for s in self.slots.iter_mut() {
            s.dirty |= reply.redraw_wins.contains(&s.id);
        }
        if reply.quit {
            self.exit = true;
        }
        // Diffed after every event, not just `win`'s: a dialog's event
        // (Open, Save As) often retitles the main window.
        let title = self.app.title();
        let caption = self.app.caption(MAIN);
        let tag = match self
            .slots
            .iter()
            .any(|s| s.id == MAIN && s.caption != caption)
        {
            true => self.tag_of(MAIN, caption),
            false => "",
        };
        let tags = self.tag_manager.clone();
        if let Some(s) = self.slot_mut(MAIN) {
            if s.title != title {
                s.title = title.clone();
                if let Some(w) = s.window.toplevel() {
                    w.set_title(title);
                }
            }
            if s.caption != caption
                && let Some(w) = s.window.toplevel()
            {
                s.caption = caption;
                send_tag(tags.as_ref(), w, tag);
            }
        }
        if let Some(text) = reply.copy {
            self.copy(text);
        }
        if let Some(clip) = reply.copy_files {
            self.copy_files(clip);
        }
        if reply.paste {
            self.paste(win);
        }
        if let Some(text) = reply.copy_primary {
            self.copy_primary(text);
        }
        if reply.paste_primary {
            self.paste_primary(win);
        }
        if reply.paste_files {
            self.paste_files(win);
        }
        for argv in reply.spawn {
            self.spawn(&reprisefs::scope::in_scope(
                &argv,
                reply.spawn_app.as_deref(),
            ));
        }
        if let Some(accept) = reply.drag_accept {
            self.drag_accept(win, accept);
        }
        if let Some(action) = reply.answer_drop {
            self.answer_drop(action);
        }
        if let Some(out) = reply.start_drag {
            self.start_drag(win, *out);
        }
        if reply.redraw_drag
            && let Some(scale) = self
                .drag_out
                .as_ref()
                .and_then(|o| o.icon.as_ref())
                .map(|i| i.scale)
        {
            self.draw_drag_icon(scale);
        }
        if reply.resize {
            self.begin_resize(win);
        }
        let fixed = self.app.fixed();
        let mut resized = false;
        if let Some(size) = reply.size
            && let Some(s) = self.slot_mut(MAIN)
        {
            s.picked = sane_size(size);
            let size = bounded(s.picked, s.bounds);
            if s.size != size {
                resized = true;
                s.size = size;
                s.buffers = [None, None];
                s.pool = None;
                s.dirty = true;
                if fixed && let Some(w) = s.window.toplevel() {
                    let (w_, h_) = (size.0 as u32, size.1 as u32);
                    w.set_min_size(Some((w_, h_)));
                    w.set_max_size(Some((w_, h_)));
                }
            }
            s.own = true;
        }
        if resized {
            self.send(MAIN, Event::Resized);
        }
        if reply.tick {
            self.arm_tick(win);
        }
        self.sync_main();
        self.sync_children();
        self.sync_flyout();
        self.sync_inhibitor();
        // An `Ask` drop cannot outlive the window answering it.
        if let Some(a) = &self.asked_drop
            && !self.slot_of_id(a.win)
            && let Some(a) = self.asked_drop.take()
        {
            a.offer.destroy();
        }
        if let Some((id, handoff)) = reply.activate
            && let Some(s) = self.slots.iter_mut().find(|s| s.id == id)
        {
            s.dirty |= s.cues.reopen(handoff.by_key);
            if let (Some(token), Some(activation)) = (handoff.token, &self.activation) {
                activation.activate::<Host>(s.window.wl_surface(), token);
            }
        }
        if let Some(id) = reply.raise {
            self.raise(id);
        }
        self.drain_head_requests();
    }

    /// Ask for a token on the last input for the keyboard's window, which `new_token` presents
    /// for window `id`.
    fn raise(&self, id: WinId) {
        let (Some(activation), Some(seat)) = (&self.activation, &self.seat) else {
            return;
        };
        let surface = self
            .focus
            .and_then(|f| self.slots.iter().find(|s| s.id == f))
            .map(|s| s.window.wl_surface().clone());
        activation.request_token(
            &self.qh,
            RequestData {
                app_id: Some(self.app.app_id()),
                seat_and_serial: Some((seat.clone(), self.last_serial)),
                surface,
                udata: Some(id),
            },
        );
    }

    fn send_tag(&mut self, window: &Window, id: WinId, caption: Caption) {
        let tag = self.tag_of(id, caption);
        if !tag.is_empty() {
            send_tag(self.tag_manager.as_ref(), window, tag);
        }
    }

    /// Window `id`'s xdg-toplevel-tag: its caption's, or `file-dialog` for a file dialog with a
    /// plain caption, whose size and place the compositor hands the next one.
    fn tag_of(&mut self, id: WinId, caption: Caption) -> &'static str {
        match caption {
            Caption::Window if self.app.drop_file_dialog(id).is_some() => "file-dialog",
            Caption::Window => "",
            Caption::Prompt => "prompt",
            Caption::Help => "help-dialog",
            Caption::Fixed => "fixed-dialog",
        }
    }

    /// Make `window` the child of the foreign `handle`, and modal to it
    /// when the program asks. Runs before the window's first commit, so
    /// it maps already centred on its owner. None when the compositor has
    /// no importer; the window then stays ownerless.
    fn import_parent(&self, handle: &str, window: &Window) -> Option<Foreign> {
        let surface = window.wl_surface();
        let imported = if let Some(v2) = &self.importer_v2 {
            let i = v2.import_toplevel(handle.to_string(), &self.qh, ForeignParent);
            i.set_parent_of(surface);
            Imported::V2(i)
        } else if let Some(v1) = &self.importer_v1 {
            let i = v1.import(handle.to_string(), &self.qh, ForeignParent);
            i.set_parent_of(surface);
            Imported::V1(i)
        } else {
            log::warn!("no xdg-foreign importer; the window stays ownerless");
            return None;
        };
        let dialog = if self.app.modal() {
            self.wm_dialog.as_ref().map(|d| {
                let x = d.get_xdg_dialog(window.xdg_toplevel(), &self.qh, NoEvents);
                x.set_modal();
                x
            })
        } else {
            None
        };
        Some(Foreign { imported, dialog })
    }

    fn make_main(&mut self) {
        let ((w, h), min) = opening_size(&*self.app);
        let surface = self.compositor.create_surface(&self.qh);
        let scaling = self.scaling(&surface);
        let window =
            self.xdg_shell
                .create_window(surface, WindowDecorations::RequestServer, &self.qh);
        let title = self.app.title();
        window.set_title(title.clone());
        window.set_app_id(self.app.app_id());
        window.set_min_size(Some((min.0 as u32, min.1 as u32)));
        if self.app.fixed() {
            window.set_max_size(Some((w as u32, h as u32)));
        }
        if let Some(handle) = self.app.foreign_parent() {
            self.foreign = self.import_parent(&handle, &window);
        }
        let caption = self.app.caption(MAIN);
        self.send_tag(&window, MAIN, caption);
        window.commit();
        self.slots.insert(
            0,
            Slot {
                id: MAIN,
                window: Shell::Toplevel(window),
                scaling,
                buffers: [None, None],
                attached: 0,
                pool: None,
                canvas: Canvas::new(0, 0),
                title,
                size: (w, h),
                origin: (0, 0),
                maximized: false,
                asked: (None, None),
                own: false,
                bounds: None,
                wanted: (w, h),
                picked: (w, h),
                activated: true,
                configured: false,
                dirty: true,
                frame_pending: false,
                mapped: false,
                modal: false,
                caption,
                access: access::SlotAccess::new(MAIN, self.a11y_bus.then_some(&self.a11y)),
                glass: None,
                glass_top: (0, 0),
                cues: Cues::opened(self.key_last),
                parent: None,
                spot: None,
            },
        );
    }

    fn sync_main(&mut self) {
        let have = self.slots.iter().any(|s| s.id == MAIN);
        match main_wanted(self.app.hidden(), have) {
            Some(true) => self.make_main(),
            Some(false) => {
                // A toplevel must not be anyone's parent when destroyed.
                for s in self.slots.iter().filter(|s| s.id != MAIN) {
                    if let Some(w) = s.window.toplevel() {
                        w.xdg_toplevel().set_parent(None);
                        w.commit();
                    }
                }
                if self.focus == Some(MAIN) {
                    self.focus = None;
                }
                self.foreign = None;
                self.drop_menus(|w| w != MAIN);
                self.slots.retain(|s| s.id != MAIN);
            }
            None => {}
        }
    }

    /// Window `id`'s min and max client size as the program asks for them now: a fixed window
    /// pinned at its size, a resizable one held at its minimum. Both stay inside the bounds.
    fn limits(&self, id: WinId) -> Option<Limits> {
        let s = self.slots.iter().find(|s| s.id == id)?;
        let (min, max) = if id == MAIN {
            if self.app.fixed() {
                (s.size, Some(s.size))
            } else {
                (self.app.min_size(MAIN).map_or(MIN_SIZE, sane_size), None)
            }
        } else {
            let c = self.app.children().into_iter().find(|c| c.id == id)?;
            let size = sane_size(c.size);
            match c.fixed {
                true => (size, Some(size)),
                false => (self.app.min_size(id).map_or(size, sane_size), None),
            }
        };
        Some((bounded(min, s.bounds), max.map(|m| bounded(m, s.bounds))))
    }

    /// The size window `id`'s program picks for it now: the main window's opening size, a
    /// child's [`Child::size`].
    fn natural(&self, id: WinId) -> Option<(i32, i32)> {
        if id == MAIN {
            return Some(opening_size(&*self.app).0);
        }
        let c = self.app.children().into_iter().find(|c| c.id == id)?;
        Some(sane_size(c.size))
    }

    /// Fit window `id` to what its program asks now, after that changed (the text size, a
    /// child's size): a fixed window takes its size; a resizable one still at its program's
    /// pick takes the size the program picks now, one the user sized keeps its size; both at
    /// least the minimum. The compositor keeps its top-left; a maximized window keeps the
    /// screen's size. A window that changed size hears `Event::Resized`.
    fn refit(&mut self, id: WinId) {
        let Some((min, max)) = self.limits(id) else {
            return;
        };
        self.declare_limits(id);
        let natural = self.natural(id);
        let Some(s) = self.slot_mut(id) else {
            return;
        };
        if s.maximized {
            return;
        }
        let natural = natural.filter(|_| at_pick(s.size, s.picked, s.bounds));
        if let Some(n) = natural {
            s.picked = n;
        }
        let size = bounded(regrown(s.size, min, max, natural), s.bounds);
        if max.is_some() {
            s.picked = size;
        }
        if size == s.size {
            return;
        }
        s.size = size;
        s.buffers = [None, None];
        s.pool = None;
        s.dirty = true;
        s.own = true;
        self.send(id, Event::Resized);
    }

    /// Tell the compositor window `id`'s [`Host::limits`].
    fn declare_limits(&mut self, id: WinId) {
        let Some((min, max)) = self.limits(id) else {
            return;
        };
        let Some(w) = self.slot_mut(id).and_then(|s| s.window.toplevel()) else {
            return;
        };
        w.set_min_size(Some((min.0 as u32, min.1 as u32)));
        if let Some(max) = max {
            w.set_max_size(Some((max.0 as u32, max.1 as u32)));
        }
    }

    fn sync_children(&mut self) {
        // `MAIN` is the host's: a `Child` claiming that id is dropped
        // here, so it can never retitle the main window or make it modal.
        let want: Vec<Child> = self
            .app
            .children()
            .into_iter()
            .filter(|c| c.id != MAIN)
            .collect();
        let have: Vec<WinId> = self
            .slots
            .iter()
            .filter(|s| s.id != MAIN && s.window.toplevel().is_some())
            .map(|s| s.id)
            .collect();
        let refs: Vec<&Child> = want.iter().collect();
        let (drop, make) = reconcile(&have, &refs);
        if self.focus.is_some_and(|f| drop.contains(&f)) {
            self.focus = None;
        }
        // A toplevel must not be anyone's parent when destroyed.
        for s in self.slots.iter_mut() {
            if s.parent.is_some_and(|p| drop.contains(&p)) {
                if let Some(w) = s.window.toplevel() {
                    w.xdg_toplevel().set_parent(None);
                    w.commit();
                }
                s.parent = None;
            }
        }
        self.drop_menus(|w| w == MAIN || !drop.contains(&w));
        self.slots.retain(|s| s.id == MAIN || !drop.contains(&s.id));
        let app_id = self.app.app_id();
        for i in make {
            let c = &want[i];
            // `reconcile` does not dedupe `want`; one surface per id.
            if self.slots.iter().any(|s| s.id == c.id) {
                continue;
            }
            let wanted = self.app.parent(c.id);
            let parent_slot = wanted.and_then(|p| self.slots.iter().find(|s| s.id == p));
            let parent = match placement(wanted, parent_slot.map(|s| s.mapped)) {
                // xdg-shell drops a parent that is not mapped yet; `draw` calls back here once
                // it is.
                Placement::Wait => continue,
                Placement::Under(_) => parent_slot
                    .and_then(|s| s.window.toplevel())
                    .map(|w| w.xdg_toplevel().clone()),
                Placement::Alone => None,
            };
            let size = sane_size(c.size);
            let surface = self.compositor.create_surface(&self.qh);
            let scaling = self.scaling(&surface);
            let window =
                self.xdg_shell
                    .create_window(surface, WindowDecorations::RequestServer, &self.qh);
            window.set_title(c.title.clone());
            window.set_app_id(app_id.clone());
            let min = match c.fixed {
                true => size,
                false => self.app.min_size(c.id).map_or(size, sane_size),
            };
            window.set_min_size(Some((min.0 as u32, min.1 as u32)));
            if c.fixed {
                window.set_max_size(Some((size.0 as u32, size.1 as u32)));
            }
            window.xdg_toplevel().set_parent(parent.as_ref());
            if wanted.is_none() {
                self.join_tabs(&window, c.id);
            }
            let caption = self.app.caption(c.id);
            self.send_tag(&window, c.id, caption);
            window.commit();
            self.slots.push(Slot {
                id: c.id,
                window: Shell::Toplevel(window),
                scaling,
                buffers: [None, None],
                attached: 0,
                pool: None,
                canvas: Canvas::new(0, 0),
                title: c.title.clone(),
                size,
                origin: (0, 0),
                maximized: false,
                asked: (None, None),
                own: false,
                bounds: None,
                wanted: size,
                picked: size,
                activated: true,
                configured: false,
                dirty: true,
                frame_pending: false,
                mapped: false,
                modal: c.modal,
                caption,
                access: access::SlotAccess::new(c.id, self.a11y_bus.then_some(&self.a11y)),
                glass: None,
                glass_top: (0, 0),
                cues: Cues::opened(self.key_last),
                parent: wanted,
                spot: None,
            });
        }
        let mut resized = Vec::new();
        for c in &want {
            self.reparent(c.id);
            let caption = self.app.caption(c.id);
            let tag = match self
                .slots
                .iter()
                .any(|s| s.id == c.id && s.caption != caption)
            {
                true => self.tag_of(c.id, caption),
                false => "",
            };
            if let Some(s) = self.slots.iter_mut().find(|s| s.id == c.id) {
                s.modal = c.modal;
                if s.wanted != sane_size(c.size) {
                    s.wanted = sane_size(c.size);
                    resized.push(c.id);
                }
                if s.title != c.title
                    && let Some(w) = s.window.toplevel()
                {
                    s.title = c.title.clone();
                    w.set_title(c.title.clone());
                }
                if s.caption != caption
                    && let Some(w) = s.window.toplevel()
                {
                    s.caption = caption;
                    send_tag(self.tag_manager.as_ref(), w, tag);
                }
            }
        }
        for id in resized {
            self.refit(id);
        }
    }

    /// Move child `id` over the window [`App::parent`] names now, once that one is mapped.
    fn reparent(&mut self, id: WinId) {
        let want = self.app.parent(id);
        let Some(s) = self.slots.iter().find(|s| s.id == id) else {
            return;
        };
        if s.parent == want {
            return;
        }
        let parent_slot = want.and_then(|p| self.slots.iter().find(|s| s.id == p));
        let parent = match placement(want, parent_slot.map(|s| s.mapped)) {
            Placement::Wait => return,
            Placement::Under(_) => parent_slot
                .and_then(|s| s.window.toplevel())
                .map(|w| w.xdg_toplevel().clone()),
            Placement::Alone => None,
        };
        if let Some(w) = s.window.toplevel() {
            w.xdg_toplevel().set_parent(parent.as_ref());
            w.commit();
        }
        if let Some(s) = self.slot_mut(id) {
            s.parent = want;
        }
    }

    /// Ask for unparented child `id`'s new `window` to join its [`App::tab_with`] window's tab
    /// group, before its first commit.
    fn join_tabs(&self, window: &Window, id: WinId) {
        let leader = self
            .app
            .tab_with(id)
            .and_then(|l| self.slots.iter().find(|s| s.id == l))
            .and_then(|s| s.window.toplevel());
        if let (Some(tabs), Some(leader)) = (&self.tab_groups, leader) {
            tabs.join(window.xdg_toplevel(), leader.xdg_toplevel());
        }
    }

    fn sync_inhibitor(&mut self) {
        let want = self
            .app
            .shortcuts_inhibited()
            .filter(|w| self.slots.iter().any(|s| s.id == *w));
        if self.inhibitor.as_ref().map(|(w, _)| *w) == want {
            return;
        }
        if let Some((_, i)) = self.inhibitor.take() {
            i.destroy();
        }
        let (Some(win), Some(manager), Some(seat)) = (want, &self.inhibit_manager, &self.seat)
        else {
            return;
        };
        let Some(s) = self.slots.iter().find(|s| s.id == win) else {
            return;
        };
        let i = manager.inhibit_shortcuts(s.window.wl_surface(), seat, &self.qh, InhibitorOf(win));
        self.inhibitor = Some((win, i));
    }

    fn sync_flyout(&mut self) {
        let have = self
            .slots
            .iter()
            .find(|s| s.window.toplevel().is_none())
            .map(|s| s.id);
        let (drop, make) = flyout_wanted(self.app.flyout(), have);
        if let Some(id) = drop {
            if self.focus == Some(id) {
                self.focus = None;
            }
            self.drop_menus(|w| w != id);
            self.slots.retain(|s| s.id != id);
        }
        if let Some(f) = make {
            self.make_flyout(f);
        }
    }

    fn make_flyout(&mut self, f: Flyout) {
        let Some(shell) = &self.layer_shell else {
            log::warn!("the compositor has no layer shell; the flyout cannot open");
            return;
        };
        let outputs: Vec<(wl_output::WlOutput, (i32, i32, i32, i32))> = self
            .output_state
            .outputs()
            .filter_map(|o| {
                let info = self.output_state.info(&o)?;
                let (x, y) = info.logical_position?;
                let (w, h) = info.logical_size?;
                Some((o, (x, y, w, h)))
            })
            .collect();
        let rects: Vec<_> = outputs.iter().map(|(_, r)| *r).collect();
        let (output, origin) = match output_containing(&rects, f.at) {
            Some(i) => (Some(&outputs[i].0), (rects[i].0, rects[i].1)),
            None => (None, (0, 0)),
        };
        let surface = self.compositor.create_surface(&self.qh);
        let scaling = self.scaling(&surface);
        let layer = shell.create_layer_surface(
            &self.qh,
            surface,
            Layer::Overlay,
            Some(self.app.app_id()),
            output,
        );
        // The whole output, taskbar included, so a click anywhere else lands here.
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        layer.commit();
        self.slots.push(Slot {
            id: f.id,
            window: Shell::Layer(layer),
            scaling,
            buffers: [None, None],
            attached: 0,
            pool: None,
            canvas: Canvas::new(0, 0),
            title: f.title.into(),
            size: (0, 0),
            origin,
            maximized: false,
            asked: (None, None),
            own: false,
            bounds: None,
            wanted: (0, 0),
            picked: (0, 0),
            activated: true,
            configured: false,
            dirty: true,
            frame_pending: false,
            mapped: false,
            modal: false,
            caption: Caption::Window,
            access: access::SlotAccess::new(f.id, self.a11y_bus.then_some(&self.a11y)),
            glass: None,
            glass_top: (0, 0),
            cues: Cues::opened(self.key_last),
            parent: None,
            spot: None,
        });
    }

    fn publish(&mut self, id: WinId) {
        let Some(i) = self.slots.iter().position(|s| s.id == id) else {
            return;
        };
        let s = &mut self.slots[i];
        if !s.access.state.dirty {
            return;
        }
        s.access.state.dirty = false;
        let title = if s.title.is_empty() {
            self.app.title()
        } else {
            s.title.clone()
        };
        let (size, modal) = (s.size, s.modal);
        self.ctx.aim(s);
        let (app, ctx) = (&mut self.app, &self.ctx);
        let menu = self.edit_menu.menu(id);
        let acc = &mut self.slots[i].access;
        let Some(adapter) = acc.adapter.as_mut() else {
            return;
        };
        adapter.update_if_active(|| {
            let mut tree = Tree::window(&title, size);
            if modal && let Some(root) = tree.get_mut(reprisekit::access::ROOT) {
                root.set_role(accesskit::Role::Dialog);
                root.set_modal();
            }
            app.access(id, &mut tree, ctx);
            if let Some(menu) = menu {
                let node = menu.access(&mut tree, reprisekit::access::ROOT);
                tree.set_focus(node);
            }
            acc.state.secret = tree
                .get(tree.focus())
                .is_some_and(|n| n.role() == accesskit::Role::PasswordInput);
            acc.state.published.update(tree)
        });
    }

    fn relook(&mut self, key: &str, value: String) {
        let text_size = self.look.text_size;
        if !self.look.set(key, value) {
            return;
        }
        let theme = self.look.theme();
        self.ctx.text = text_for(&theme.fonts);
        publish_theme(&theme);
        self.ctx.theme = sized(theme, &*self.app, self.look.text_size);
        self.ctx.text_size = self.look.text_size;
        if key == reprisebus::appearance::ICON_THEME {
            let scale = self.ctx.icons.scale();
            self.ctx.icons = self.look.icons();
            self.ctx.icons.set_scale(scale);
        }
        for s in self.slots.iter_mut() {
            s.dirty = true;
        }
        self.menu_relooked();
        // Forms and the windows that hold them grow with the text size: each window hears it as
        // a resize and answers with its new size, then grows to what its program asks now.
        if self.look.text_size != text_size {
            let ids: Vec<WinId> = self.slots.iter().map(|s| s.id).collect();
            for &id in &ids {
                self.send(id, Event::Resized);
            }
            for id in ids {
                self.refit(id);
            }
        }
    }

    fn access_msg(&mut self, msg: access::Msg) {
        match msg {
            access::Msg::Activate(win) => {
                if let Some(s) = self.slot_mut(win) {
                    s.access.state.published.reset();
                    s.access.state.dirty = true;
                    s.access.state.active = true;
                }
                self.key_echo
                    .get_or_insert_with(reprisebus::a11y::KeyEcho::spawn);
                self.publish(win);
            }
            access::Msg::Deactivate(win) => {
                if let Some(s) = self.slot_mut(win) {
                    s.access.state.published.reset();
                    s.access.state.active = false;
                }
            }
            access::Msg::Act(win, req) => {
                let target = match self.reader_route(win, req) {
                    Route::Reveal { reveal, then } => {
                        self.reader_reveal(win, reveal);
                        if !then {
                            return;
                        }
                        self.publish(win);
                        match self.reader_route(win, req) {
                            Route::Act(t) => t,
                            _ => return,
                        }
                    }
                    Route::Act(t) => t,
                    Route::None => return,
                };
                let Some(g) = reprisekit::access::gesture(req.act, target) else {
                    return;
                };
                if let Gesture::Focus(at) = g {
                    self.reader_focus(win, at);
                    return;
                }
                let apart = self.ctx.theme.metrics.double_click_ms.max(0) as u32 + 1000;
                self.press_time = self.press_time.wrapping_add(apart);
                for input in access::reader_input(g, self.press_time) {
                    self.pointer(win, input);
                }
            }
        }
    }

    /// How a reader's `req` in window `win` is carried out, on the tree last published there.
    fn reader_route(&mut self, win: WinId, req: reprisekit::access::Request) -> Route {
        (self.slot_mut(win))
            .filter(|s| s.access.state.active)
            .map_or(Route::None, |s| s.access.state.published.route(req))
    }

    /// Scroll the list in window `win` so the row `r` names shows.
    fn reader_reveal(&mut self, win: WinId, r: Reveal) {
        let at = (r.at.0 as i32, r.at.1 as i32);
        self.reader_act(win, Reader::Reveal { at, row: r.row });
    }

    /// Give the keyboard to the control at `at` in window `win`, pressing nothing.
    fn reader_focus(&mut self, win: WinId, at: (f64, f64)) {
        self.reader_act(win, Reader::Focus((at.0 as i32, at.1 as i32)));
    }

    /// Hand `act` to the program ([`App::reader`]); when it reports a change the window and its
    /// tree are drawn again.
    fn reader_act(&mut self, win: WinId, act: Reader) {
        let Some(s) = self.slots.iter().find(|s| s.id == win) else {
            return;
        };
        self.ctx.aim(s);
        if self.app.reader(win, act, &self.ctx)
            && let Some(s) = self.slot_mut(win)
        {
            s.dirty = true;
            s.access.state.dirty = true;
        }
    }

    fn key_input(&self, event: &KeyEvent) -> KeyInput {
        let sym = event.keysym.raw();
        let utf8 = event.utf8.clone();
        let edit = key_for(sym, self.mods.ctrl, self.mods.shift, utf8.as_deref());
        KeyInput {
            sym,
            utf8,
            mods: self.seat_mods(),
            edit,
        }
    }

    fn seat_mods(&self) -> Mods {
        Mods {
            ctrl: self.mods.ctrl,
            shift: self.mods.shift,
            alt: self.mods.alt,
        }
    }

    fn echo_key(&self, event: &KeyEvent, pressed: bool) {
        let Some(echo) = &self.key_echo else {
            return;
        };
        if !self.slots.iter().any(|s| s.access.state.active) {
            return;
        }
        let m = &self.mods;
        let mods = u32::from(m.shift)
            | u32::from(m.caps_lock) << 1
            | u32::from(m.ctrl) << 2
            | u32::from(m.alt) << 3
            | u32::from(m.num_lock) << 4
            | u32::from(m.logo) << 6;
        let typed = event
            .utf8
            .as_deref()
            .filter(|t| t.chars().all(|c| !c.is_control()) && !t.is_empty());
        // What goes into a password field stays in the program; the reader
        // hears the bullets the field reports instead.
        let secret = self
            .focus
            .and_then(|f| self.slots.iter().find(|s| s.id == f))
            .is_some_and(|s| s.access.state.secret);
        if secret && typed.is_some() {
            return;
        }
        let text = match typed {
            Some(t) => t.to_string(),
            None => event
                .keysym
                .name()
                .unwrap_or_default()
                .trim_start_matches("XK_")
                .to_string(),
        };
        echo.send(reprisebus::a11y::KeyNote {
            pressed,
            keysym: event.keysym.raw(),
            keycode: event.raw_code + 8,
            mods,
            time: event.time,
            text: text.into(),
            is_text: typed.is_some(),
        });
    }

    fn key(&mut self, mut event: KeyEvent) {
        self.echo_key(&event, true);
        let input = self.key_input(&event);
        event.utf8.zeroize();
        self.ctx.caret_on = true;
        self.key_last = true;
        let win = self.focus.unwrap_or(MAIN);
        let alt = self.mods.alt || matches!(event.keysym, Keysym::Alt_L | Keysym::Alt_R);
        if let Some(s) = self.slot_mut(win) {
            let was = s.cues.keys;
            s.cues.key(alt);
            s.dirty |= s.cues.keys != was;
        }
        self.send(win, Event::Key(input));
    }

    fn key_up(&mut self, mut event: KeyEvent) {
        self.echo_key(&event, false);
        let input = self.key_input(&event);
        event.utf8.zeroize();
        let win = self.focus.unwrap_or(MAIN);
        self.send(win, Event::KeyUp(input));
    }

    fn pointer(&mut self, win: WinId, input: PointerInput) {
        self.send(win, Event::Pointer(input));
    }

    fn copy(&mut self, text: String) {
        let (Some(manager), Some(device)) = (&self.data_device_manager, &self.data_device) else {
            return;
        };
        let source =
            manager.create_copy_paste_source(&self.qh, [TEXT_MIME, "text/plain", "UTF8_STRING"]);
        source.set_selection(device, self.last_serial);
        self.copy_text = text.into_bytes().into();
        self.copy_clip = None;
        self.copy_source = Some(source);
    }

    fn copy_primary(&mut self, text: String) {
        self.primary.copy(&self.qh, text, self.last_serial);
    }

    fn paste_primary(&mut self, win: WinId) {
        if let Some(pipe) = self.primary.receive() {
            self.read_offer(win, pipe, |buf| Event::PastedPrimary(buf.into_text()));
        }
    }

    fn copy_files(&mut self, clip: FileClip) {
        let (Some(manager), Some(device)) = (&self.data_device_manager, &self.data_device) else {
            return;
        };
        let Some(offers) = reprisefs::fileclip::offers(&clip) else {
            return;
        };
        let source = manager.create_copy_paste_source(&self.qh, offers.iter().map(|(m, _)| *m));
        source.set_selection(device, self.last_serial);
        self.copy_clip = Some(offers);
        self.copy_text = Arc::default();
        self.copy_source = Some(source);
    }

    fn paste(&mut self, win: WinId) {
        let Some(device) = &self.data_device else {
            return;
        };
        let Some(offer) = device.data().selection_offer() else {
            return;
        };
        let Some(mime) = offer.with_mime_types(text_mime) else {
            return;
        };
        let Ok(read_pipe) = offer.receive(mime.to_string()) else {
            return;
        };
        self.read_offer(win, read_pipe, move |buf| {
            Event::Pasted(pasted_text(mime, buf))
        });
    }

    /// Read the clipboard's files for `win`: a cut marked in [`reprisefs::fileclip::KDE_CUT`]
    /// first, then the files.
    fn paste_files(&mut self, win: WinId) {
        use reprisefs::fileclip;
        let Some(offer) = self.selection_offer() else {
            return;
        };
        let Some(source) = offer.with_mime_types(fileclip::source) else {
            return;
        };
        if !source.kde_cut {
            return self.paste_files_as(win, source.files, false);
        }
        let Ok(read_pipe) = offer.receive(fileclip::KDE_CUT.to_string()) else {
            return;
        };
        read_capped(
            &self.loop_handle,
            read_pipe,
            reprisefs::pipe::PASTE_TIMEOUT,
            move |host: &mut Host, buf| {
                let cut = buf.is_some_and(|b| fileclip::is_cut_marker(b.bytes()));
                host.paste_files_as(win, source.files, cut);
            },
        );
    }

    fn paste_files_as(&mut self, win: WinId, mime: &'static str, kde_cut: bool) {
        let Some(offer) = self.selection_offer() else {
            return;
        };
        let Ok(read_pipe) = offer.receive(mime.to_string()) else {
            return;
        };
        self.read_offer(win, read_pipe, move |buf| {
            let mut clip = reprisefs::fileclip::decode(mime, buf.bytes());
            clip.cut |= kde_cut;
            Event::PastedFiles(clip)
        });
    }

    fn selection_offer(&self) -> Option<SelectionOffer> {
        self.data_device.as_ref()?.data().selection_offer()
    }

    fn read_offer(
        &mut self,
        win: WinId,
        read_pipe: ReadPipe,
        make_event: impl FnOnce(PasteBuf) -> Event + 'static,
    ) {
        read_capped(
            &self.loop_handle,
            read_pipe,
            reprisefs::pipe::PASTE_TIMEOUT,
            move |host: &mut Host, buf| {
                if let Some(buf) = buf {
                    host.send(win, make_event(buf));
                }
            },
        );
    }

    fn spawn(&mut self, argv: &[OsString]) {
        if let Err(e) = spawn_in(argv, None, None) {
            log::warn!("spawn {argv:?}: {e}");
        }
    }

    /// Set slot `i`'s blur region to its top `rows.1` rows across its `rows.0` width, for the
    /// commit that follows; none at 0.
    fn set_glass_top(&mut self, i: usize, rows: (i32, i32), qh: &QueueHandle<Host>) {
        let Some(effects) = &self.effects else {
            return;
        };
        let s = &mut self.slots[i];
        let had = s.glass_top.1 > 0;
        if s.glass_top == rows || (!had && rows.1 == 0) {
            return;
        }
        s.glass_top = rows;
        let surface = s.window.wl_surface();
        let effect = s
            .glass
            .get_or_insert_with(|| effects.get_background_effect(surface, qh, NoEvents));
        if rows.1 == 0 {
            effect.set_blur_region(None);
            return;
        }
        match smithay_client_toolkit::compositor::Region::new(&self.compositor) {
            Ok(region) => {
                region.add(0, 0, rows.0, rows.1);
                effect.set_blur_region(Some(region.wl_region()));
            }
            Err(e) => log::warn!("blur region: {e}"),
        }
    }

    fn draw(&mut self, id: WinId, qh: &QueueHandle<Host>) {
        let Some(i) = self.slots.iter().position(|s| s.id == id) else {
            return;
        };
        if self.slots[i].frame_pending {
            self.slots[i].dirty = true;
            return;
        }
        let (w, h) = self.slots[i].size;
        let sc = &mut self.slots[i].scaling;
        let scale = draw_scale(sc.fraction_120, sc.buffer_scale, (w, h));
        if std::mem::replace(&mut sc.drawn, scale) != scale {
            self.slots[i].buffers = [None, None];
            self.slots[i].pool = None;
        }
        self.ctx.icons.set_scale(scale);
        let (bw, bh) = (w * scale, h * scale);
        let stride = bw * 4;
        // Cleared: an app may leave a region alone and expect it transparent.
        let mut canvas = std::mem::replace(&mut self.slots[i].canvas, Canvas::new(0, 0));
        let fresh = (canvas.width, canvas.height, canvas.scale()) == (w as u32, h as u32, scale);
        if !fresh {
            canvas = Canvas::with_scale(w as u32, h as u32, scale);
        } else if !self.app.keeps_canvas(id) {
            canvas.pixels.fill(0);
        }
        self.ctx.size = (w, h);
        self.ctx.maximized = self.slots[i].maximized;
        self.ctx.at_pick = at_pick((w, h), self.slots[i].picked, self.slots[i].bounds);
        self.ctx.origin = self.slots[i].origin;
        // Only the active window draws as active (a dialog's default
        // button): a toplevel as the compositor
        // says, a flyout while it has the keyboard.
        self.ctx.theme.active = match self.slots[i].window {
            Shell::Toplevel(_) => self.slots[i].activated,
            Shell::Layer(_) => self.focus == Some(id),
        };
        self.ctx.theme.access_keys = self.slots[i].cues.keys;
        let preedit = self
            .text_input
            .as_ref()
            .filter(|t| t.entered == Some(id))
            .and_then(|t| t.preedit().cloned());
        canvas.set_preedit(preedit);
        filedlg::follow_scale(&mut *self.app, id, scale, &mut self.ctx);
        self.app.follow_scale(id, scale, &mut self.ctx);
        let paste = self.edit_menu.is_pending(id) && self.clipboard_text();
        let within = self.menu_within((w, h));
        let app = &mut self.app;
        let menu = &mut self.edit_menu;
        let spot = self.ctx.at_scale(scale, |ctx| {
            app.draw(id, &mut canvas, ctx);
            // A form's label tip goes over everything; a kept canvas would keep its pixels.
            if app.keeps_canvas(id) {
                canvas.take_tip();
            } else if let Some(r) = form::draw_tip(&mut canvas, &ctx.text, &ctx.forms()) {
                canvas.widen_damage(r);
            }
            let spot = canvas.take_spot();
            let cx = MenuCtx {
                text: &ctx.text,
                theme: &ctx.theme,
                px: ctx.theme.fonts.size_px,
                within,
            };
            menu.drawn(id, spot.as_ref(), paste, &cx);
            spot
        });
        self.slots[i].spot = spot.clone();
        let glass_top = canvas.take_glass_top();
        let glass_top = match reprisekit::look::client_glass(&self.ctx.theme, true) {
            Some(_) => glass_top.clamp(0, h),
            None => 0,
        };
        self.ctx.theme.active = true;
        self.ctx.theme.access_keys = false;
        self.set_glass_top(i, (w, glass_top), qh);
        let damage = canvas.take_damage();
        if let Some(t) = self.text_input.as_mut() {
            t.sync(id, spot);
        }
        // After the draw: the tree reads layout the draw cached.
        let mapped = self.slots[i].mapped;
        self.slots[i].access.state.after_draw(mapped);
        self.publish(id);
        let shm = &self.shm;
        let slot = &mut self.slots[i];
        let pool = slot.pool.get_or_insert_with(|| {
            // SCTK rounds a slot up to 64 bytes; a pool a byte short of
            // that doubles itself on the first allocation.
            let len = ((bh * stride) as usize + 63) & !63;
            SlotPool::new(len.max(64), shm).expect("shm pool")
        });
        let free = slot
            .buffers
            .each_ref()
            .map(|b| b.as_ref().map(|b| b.canvas(pool).is_some()));
        let (k, make) = next_buffer(free, slot.attached);
        if make {
            let (buffer, _) = pool
                .create_buffer(bw, bh, stride, wl_shm::Format::Argb8888)
                .expect("create buffer");
            slot.buffers[k] = Some(buffer);
        }
        let buffer = slot.buffers[k].as_ref().expect("buffer");
        let target = pool.canvas(buffer).expect("a released buffer");
        copy_pixels(target, &canvas.pixels);
        let s = &mut self.slots[i];
        s.attached = k;
        s.canvas = canvas;
        let surface = s.window.wl_surface();
        match &s.scaling.viewport {
            Some(v) => v.set_destination(w, h),
            None => surface.set_buffer_scale(scale),
        }
        let keeps = self.app.keeps_canvas(id);
        for (x, y, w, h) in surface_damage(damage, fresh, keeps, scale, (bw, bh)) {
            surface.damage_buffer(x, y, w, h);
        }
        surface.frame(qh, FrameCallbackData(surface.clone()));
        s.frame_pending = true;
        s.buffers[k]
            .as_ref()
            .expect("buffer")
            .attach_to(surface)
            .expect("attach");
        s.window.commit();
        if !std::mem::replace(&mut s.mapped, true) {
            self.sync_children();
        }
    }
}

/// Read `pipe` on the loop until its writer closes it, then hand the
/// bytes to `done`. The writer is any client: an offer past the cap, or
/// one still open after `timeout` (written to or not), is dropped and
/// `done` gets None. `done` runs once, unless the loop refuses the pipe.
fn read_capped<D: 'static>(
    handle: &LoopHandle<'static, D>,
    pipe: ReadPipe,
    timeout: Duration,
    done: impl FnOnce(&mut D, Option<PasteBuf>) + 'static,
) {
    type Done<D> = Rc<RefCell<Option<Box<dyn FnOnce(&mut D, Option<PasteBuf>)>>>>;
    let done: Done<D> = Rc::new(RefCell::new(Some(Box::new(done))));
    let finish = |done: &Done<D>, data: &mut D, buf: Option<PasteBuf>| {
        let f = done.borrow_mut().take();
        if let Some(f) = f {
            f(data, buf);
        }
    };
    let mut buf = PasteBuf::default();
    let on_read = done.clone();
    let token = handle.insert_source(pipe, move |_, pipe, data: &mut D| {
        let mut chunk = Zeroizing::new([0u8; 4096]);
        match (&**pipe).read(&mut chunk[..]) {
            Ok(0) | Err(_) => {
                finish(&on_read, data, Some(std::mem::take(&mut buf)));
                PostAction::Remove
            }
            Ok(n) if buf.push(&chunk[..n]) => PostAction::Continue,
            Ok(_) => {
                log::warn!("paste: offer too large; dropped");
                finish(&on_read, data, None);
                PostAction::Remove
            }
        }
    });
    let Ok(token) = token else {
        return;
    };
    let remover = handle.clone();
    let _ = handle.insert_source(Timer::from_duration(timeout), move |_, _, data| {
        remover.remove(token);
        finish(&done, data, None);
        TimeoutAction::Drop
    });
}

/// Which of a slot's two buffers the next frame goes into, and whether to
/// make it. `free[k]` is `None` for a buffer not made yet, else whether the
/// compositor released it. Reuse a released one, else make a missing one,
/// else replace the one not attached last.
fn next_buffer(free: [Option<bool>; 2], attached: usize) -> (usize, bool) {
    if let Some(k) = free.iter().position(|f| *f == Some(true)) {
        (k, false)
    } else if let Some(k) = free.iter().position(Option::is_none) {
        (k, true)
    } else {
        (1 - attached, true)
    }
}

/// Reported logical rects as buffer-px `(x, y, width, height)` at `scale`.
fn buffer_damage(rects: &[Rect], scale: i32) -> Vec<(i32, i32, i32, i32)> {
    rects
        .iter()
        .map(|r| (r.x * scale, r.y * scale, r.width * scale, r.height * scale))
        .collect()
}

/// The buffer-px rects to damage: the reported ones only when the canvas kept last frame's pixels
/// (`fresh` and the app `keeps` it), else the whole `buffer`.
fn surface_damage(
    damage: Option<Vec<Rect>>,
    fresh: bool,
    keeps: bool,
    scale: i32,
    buffer: (i32, i32),
) -> Vec<(i32, i32, i32, i32)> {
    match (fresh && keeps).then_some(damage).flatten() {
        Some(rects) => buffer_damage(&rects, scale),
        None => vec![(0, 0, buffer.0, buffer.1)],
    }
}

fn copy_pixels(target: &mut [u8], pixels: &[u32]) {
    let n = pixels.len().min(target.len() / 4);
    #[cfg(target_endian = "little")]
    {
        // SAFETY: `u32` has no padding and any bit pattern is a valid
        // byte; the slice covers exactly `n` words of `pixels`.
        let bytes = unsafe { std::slice::from_raw_parts(pixels.as_ptr() as *const u8, n * 4) };
        target[..n * 4].copy_from_slice(bytes);
    }
    #[cfg(not(target_endian = "little"))]
    for (dst, px) in target.as_chunks_mut::<4>().0.iter_mut().zip(&pixels[..n]) {
        *dst = px.to_le_bytes();
    }
}

impl CompositorHandler for Host {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        factor: i32,
    ) {
        if !self.rescale_drag_icon(surface, factor) {
            self.rescale(surface, None, Some(factor));
        }
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        let Some(id) = self.slot_of(surface) else {
            return;
        };
        let Some(s) = self.slot_mut(id) else {
            return;
        };
        s.frame_pending = false;
        if s.dirty && s.configured {
            s.dirty = false;
            self.draw(id, qh);
        }
    }
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        output: &wl_output::WlOutput,
    ) {
        let info = self.output_state.info(output);
        if let Some(size) = info.as_ref().and_then(|i| i.logical_size)
            && self
                .slots
                .iter()
                .any(|s| s.id == MAIN && s.window.wl_surface() == surface)
        {
            CAPTION_FIT.with_borrow_mut(|(_, w)| *w = Some(size.0));
            self.send(MAIN, Event::Output(size));
        }
        let Some((x, y)) = info.and_then(|i| i.logical_position) else {
            return;
        };
        if let Some(s) = self
            .slots
            .iter_mut()
            .find(|s| s.window.toplevel().is_none() && s.window.wl_surface() == surface)
            && s.origin != (x, y)
        {
            s.origin = (x, y);
            s.dirty = true;
        }
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Host {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl WindowHandler for Host {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, window: &Window) {
        let Some(id) = self.slot_of(window.wl_surface()) else {
            return;
        };
        self.send(id, Event::CloseRequest);
    }

    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        window: &Window,
        configure: WindowConfigure,
        _: u32,
    ) {
        let Some(id) = self.slot_of(window.wl_surface()) else {
            return;
        };
        let asked = (
            configure.new_size.0.map(|v| sane_side(v.get().into())),
            configure.new_size.1.map(|v| sane_side(v.get().into())),
        );
        let bounds = configure
            .suggested_bounds
            .filter(|&(w, h)| w > 0 && h > 0)
            .map(|(w, h)| sane_size((w as i32, h as i32)));
        let s = self.slot_mut(id).expect("slot");
        let (size, own) = configured_size(s.size, asked, s.asked, s.own, configure.is_resizing());
        let moved_bounds = bounds.is_some() && bounds != s.bounds;
        let size = match moved_bounds && size == s.size {
            true => rebounded(size, s.picked, s.bounds, bounds),
            false => size,
        };
        if bounds.is_some() {
            s.bounds = bounds;
        }
        let size = held_size(size, asked, s.bounds);
        s.asked = asked;
        s.own = own;
        if s.mapped {
            s.activated = configure.is_activated();
        }
        if moved_bounds {
            self.declare_limits(id);
        }
        self.configured(id, size, configure.is_maximized(), qh);
    }
}

impl LayerShellHandler for Host {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if let Some(id) = self.slot_of(layer.wl_surface()) {
            self.send(id, Event::CloseRequest);
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let Some(id) = self.slot_of(layer.wl_surface()) else {
            return;
        };
        let (w, h) = configure.new_size;
        let size = (sane_side(w.into()), sane_side(h.into()));
        self.configured(id, size, false, qh);
    }
}

impl Host {
    fn configured(
        &mut self,
        id: WinId,
        (w, h): (i32, i32),
        maximized: bool,
        qh: &QueueHandle<Host>,
    ) {
        let Some(s) = self.slot_mut(id) else {
            return;
        };
        s.maximized = maximized;
        let changed = (w, h) != s.size || !s.configured;
        if changed {
            s.size = (w, h);
            s.buffers = [None, None];
            s.pool = None;
        }
        s.configured = true;
        if changed {
            self.send(id, Event::Resized);
        }
        self.draw(id, qh);
    }
}

impl SeatHandler for Host {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.seat = Some(seat.clone());
            let keyboard = self
                .seat_state
                .get_keyboard_with_repeat(
                    qh,
                    &seat,
                    None,
                    self.loop_handle.clone(),
                    Box::new(|host: &mut Host, _, event| host.key(event)),
                )
                .expect("keyboard");
            self.keyboard = Some(keyboard);
            if let Some(manager) = &self.text_input_manager
                && self.text_input.is_none()
            {
                let ti = manager.get_text_input(&seat, qh, TextInputOf);
                self.text_input = Some(textinput::TextInput::new(ti));
            }
            if let Some(manager) = &self.data_device_manager
                && self.data_device.is_none()
            {
                self.data_device = Some(manager.get_data_device(qh, &seat));
            }
            self.primary.attach(qh, &seat);
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard
            && let Some(k) = self.keyboard.take()
        {
            k.release();
            if let Some(t) = self.text_input.take() {
                t.destroy();
            }
        }
        if capability == Capability::Pointer
            && let Some(p) = self.pointer.take()
        {
            p.release();
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl Host {
    /// Window `id` lost the keyboard.
    fn lose_keyboard(&mut self, id: WinId) {
        if self.focus == Some(id) {
            self.focus = None;
        }
        if let Some(s) = self.slot_mut(id) {
            if let Some(a) = s.access.adapter.as_mut() {
                a.update_window_focus_state(false);
            }
            s.dirty |= s.cues.blur();
        }
        self.redraw_if_active_shows(id);
        self.send(id, Event::Focus(false));
    }

    /// After a turn with a deferred keyboard leave: the focused window lost the keyboard
    /// unless it went to one of its own surfaces.
    fn settle_keyboard(&mut self) {
        if !std::mem::take(&mut self.kb_settle) {
            return;
        }
        if let Some(f) = self.focus
            && self.kb_on.as_ref().map(|(_, id)| *id) != Some(f)
        {
            self.lose_keyboard(f);
        }
    }
}

impl KeyboardHandler for Host {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        serial: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
        self.last_serial = serial;
        let Some(id) = self.slot_of(surface).or_else(|| self.menu_owner(surface)) else {
            return;
        };
        self.kb_on = Some((surface.clone(), id));
        // From a window to its menu or back: the window keeps the keyboard.
        if self.focus == Some(id) {
            return;
        }
        self.focus = Some(id);
        if let Some(s) = self.slot_mut(id)
            && let Some(a) = s.access.adapter.as_mut()
        {
            a.update_window_focus_state(true);
        }
        self.redraw_if_active_shows(id);
        self.send(id, Event::Focus(true));
    }
    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        let was_on = match self.kb_on.take() {
            Some((on, id)) if &on == surface => Some(id),
            other => {
                self.kb_on = other;
                None
            }
        };
        let window = self.slot_of(surface);
        // A menu popup, open or just destroyed, or a window handing the keyboard to its menu:
        // where it went is known once the turn's events are in.
        let to_menu = window.is_some_and(|id| self.edit_menu.is_open(id));
        if window.is_none() && was_on.is_some() || to_menu {
            self.kb_settle = true;
            return;
        }
        if let Some(id) = window {
            self.lose_keyboard(id);
        }
    }
    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        serial: u32,
        event: KeyEvent,
    ) {
        self.last_serial = serial;
        self.input_serial = serial;
        self.key(event);
    }
    fn repeat_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.key(event);
    }
    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.key_up(event);
    }
    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: Modifiers,
        _: RawModifiers,
        _: u32,
    ) {
        self.mods = modifiers;
    }
}

impl PointerHandler for Host {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            let local = (event.position.0 as i32, event.position.1 as i32);
            let Some((id, (x, y))) = self
                .slot_of(&event.surface)
                .map(|id| (id, local))
                .or_else(|| self.menu_point(&event.surface, local))
            else {
                continue;
            };
            self.pointer_pos = (x, y);
            match event.kind {
                PointerEventKind::Enter { .. } => self.pointer_in = Some(id),
                PointerEventKind::Leave { .. } if self.pointer_in == Some(id) => {
                    self.pointer_in = None
                }
                _ => {}
            }
            let input = match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    PointerInput::Motion { x, y }
                }
                PointerEventKind::Leave { .. } => PointerInput::Leave,
                PointerEventKind::Press {
                    button,
                    serial,
                    time,
                } => {
                    self.last_serial = serial;
                    self.press_serial = serial;
                    self.input_serial = serial;
                    self.press_time = time;
                    self.key_last = false;
                    PointerInput::Press {
                        x,
                        y,
                        button,
                        time,
                        mods: self.seat_mods(),
                    }
                }
                PointerEventKind::Release { button, .. } => PointerInput::Release {
                    x,
                    y,
                    button,
                    mods: self.seat_mods(),
                },
                PointerEventKind::Axis { vertical, .. } => {
                    let rows = if vertical.discrete != 0 {
                        vertical.discrete * reprisekit::pointer::WHEEL_ROWS
                    } else {
                        (vertical.absolute / 5.0) as i32
                    };
                    if rows == 0 {
                        continue;
                    }
                    PointerInput::Scroll { x, y, rows }
                }
            };
            self.pointer(id, input);
        }
    }
}

impl ShmHandler for Host {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

struct Outgoing {
    win: WinId,
    paths: Vec<PathBuf>,
    action: Option<DragAction>,
    dropped: bool,
    transfers: reprisebus::filetransfer::Transfers,
    icon: Option<DragIcon>,
    source: DragSource,
}

struct DragIcon {
    surface: wl_surface::WlSurface,
    buffer: Buffer,
    _pool: SlotPool,
    scale: i32,
    grab: (i32, i32),
}

impl Drop for DragIcon {
    fn drop(&mut self) {
        self.surface.destroy();
    }
}

/// An shm buffer holding `image`'s device pixels.
fn icon_buffer(image: &Canvas, shm: &Shm) -> Option<(Buffer, SlotPool)> {
    let (dw, dh) = image.device_size();
    if dw == 0 || dh == 0 || dw > MAX_SIDE as u32 || dh > MAX_SIDE as u32 {
        return None;
    }
    let (dw, dh) = (dw as i32, dh as i32);
    let mut pool = SlotPool::new((dw * dh * 4) as usize, shm).ok()?;
    let (buffer, pixels) = pool
        .create_buffer(dw, dh, dw * 4, wl_shm::Format::Argb8888)
        .ok()?;
    copy_pixels(pixels, &image.pixels);
    Some((buffer, pool))
}

impl DragIcon {
    /// Shows a buffer drawn at another scale; the grab, in logical px, stays where it was.
    fn redraw(&self) {
        self.surface.set_buffer_scale(self.scale);
        if self.buffer.attach_to(&self.surface).is_err() {
            return;
        }
        self.surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
        self.surface.commit();
    }

    /// Runs after `start_drag`, so the compositor's icon sees the offset commit.
    fn show(&self) {
        let (gx, gy) = self.grab;
        self.surface.set_buffer_scale(self.scale);
        if self.surface.version() >= 5 {
            if self.buffer.attach_to(&self.surface).is_err() {
                return;
            }
            self.surface.offset(-gx, -gy);
        } else {
            self.surface.attach(Some(self.buffer.wl_buffer()), -gx, -gy);
        }
        self.surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
        self.surface.commit();
    }
}

struct Asked {
    win: WinId,
    offer: WlDataOffer,
    source: Actions,
}

/// So the dragging program never waits on a menu nobody answers.
const ASK_TIMEOUT: Duration = Duration::from_secs(60);

type OnLoop = Box<dyn FnOnce(&mut Host) + Send>;

/// A window's min client size and, for a fixed one, its max.
type Limits = ((i32, i32), Option<(i32, i32)>);

struct Incoming {
    win: WinId,
    /// Held raw: SCTK's `DragOffer` refuses requests once the drag has
    /// left, and the compositor sends `leave` right after `drop`.
    offer: WlDataOffer,
    /// The enter's serial, which every `accept` names.
    serial: u32,
    mimes: Vec<String>,
    pos: (i32, i32),
    /// What the dragging program allows.
    source: Actions,
    action: Option<DragAction>,
    accept: Option<(Accept, &'static str)>,
    /// Whether the files read ahead of the drop were there; None until
    /// that read lands. It picks the mime a `DropKind::Uris` drop is read
    /// in (`dnd::drop_mime`).
    files: Option<bool>,
    /// Its files come through the document portal (`dnd::through_portal`):
    /// a sandboxed program's drag. They are never moved.
    portal: bool,
}

impl Host {
    fn begin_resize(&mut self, win: WinId) {
        let Some(seat) = &self.seat else {
            return;
        };
        if let Some(w) = self
            .slots
            .iter()
            .find(|s| s.id == win)
            .and_then(|s| s.window.toplevel())
        {
            w.resize(
                seat,
                self.press_serial,
                xdg_toplevel::ResizeEdge::BottomRight,
            );
        }
    }

    fn start_drag(&mut self, win: WinId, out: DragOut) {
        if let Some(old) = self.drag_out.take() {
            self.drag_ended(old, false);
        }
        let (Some(manager), Some(device)) = (&self.data_device_manager, &self.data_device) else {
            return;
        };
        let Some(origin) = self
            .slots
            .iter()
            .find(|s| s.id == win)
            .map(|s| s.window.wl_surface().clone())
        else {
            return;
        };
        let transfers = reprisebus::filetransfer::Transfers::for_drag();
        let mimes = dnd::source_mimes(transfers.offered());
        let source = manager.create_drag_and_drop_source(&self.qh, mimes, DndAction::all());
        let icon = self.drag_icon(&out.image, out.grab);
        source.start_drag(
            device,
            &origin,
            icon.as_ref().map(|i| &i.surface),
            self.press_serial,
        );
        if let Some(i) = &icon {
            i.show();
        }
        self.drag_out = Some(Outgoing {
            win,
            paths: out.paths,
            action: None,
            dropped: false,
            transfers,
            icon,
            source,
        });
    }

    fn drag_icon(&self, image: &Canvas, grab: (i32, i32)) -> Option<DragIcon> {
        let (buffer, pool) = icon_buffer(image, &self.shm)?;
        Some(DragIcon {
            surface: self.compositor.create_surface(&self.qh),
            buffer,
            _pool: pool,
            scale: image.scale(),
            grab,
        })
    }

    /// The compositor shows drag icon `surface` at `factor` now: the program draws its image
    /// again at that scale. False when `surface` is no drag icon.
    fn rescale_drag_icon(&mut self, surface: &wl_surface::WlSurface, factor: i32) -> bool {
        let Some(out) = &self.drag_out else {
            return false;
        };
        let Some(icon) = out.icon.as_ref().filter(|i| &i.surface == surface) else {
            return false;
        };
        let scale = factor.max(1);
        if icon.scale != scale {
            self.draw_drag_icon(scale);
        }
        true
    }

    /// The running drag's icon shows the program's image drawn again at `scale`.
    fn draw_drag_icon(&mut self, scale: i32) {
        let Some(win) = self.drag_out.as_ref().map(|o| o.win) else {
            return;
        };
        self.ctx.icons.set_scale(scale);
        let image = self.app.drag_image(win, scale, &mut self.ctx);
        if let Some(image) = image.filter(|i| i.scale() == scale)
            && let Some((buffer, pool)) = icon_buffer(&image, &self.shm)
            && let Some(icon) = self.drag_out.as_mut().and_then(|o| o.icon.as_mut())
        {
            // The old buffer goes once the new one is committed.
            let _old = (
                std::mem::replace(&mut icon.buffer, buffer),
                std::mem::replace(&mut icon._pool, pool),
            );
            icon.scale = scale;
            icon.redraw();
        }
    }

    fn drag_ended(&mut self, out: Outgoing, finished: bool) {
        let dropped = out.dropped;
        let action = if finished { out.action } else { None };
        let win = out.win;
        drop(out);
        self.send(win, Event::DragEnd { dropped, action });
    }

    fn drag_of(&mut self, source: &WlDataSource) -> Option<&mut Outgoing> {
        self.drag_out
            .as_mut()
            .filter(|o| o.source.inner() == source)
    }

    /// The program answered a drag over window `win`. An answer that
    /// changes nothing is not resent: `set_actions` makes the compositor
    /// pick the action again.
    fn drag_accept(&mut self, win: WinId, accept: Option<Accept>) {
        let Some(inc) = self.drag_in.as_mut().filter(|i| i.win == win) else {
            return;
        };
        let files = inc.files == Some(true);
        let answer = accept.and_then(|a| {
            let mime = dnd::drop_mime(&inc.mimes, a.kind, files, inc.portal)?;
            let a = Accept {
                actions: a.actions.read_as(mime == dnd::FILE_TRANSFER),
                ..a
            };
            (a.actions != Actions::default()).then_some((a, mime))
        });
        if answer == inc.accept {
            return;
        }
        inc.accept = answer;
        let (mime, (actions, preferred)) = match answer {
            Some((a, mime)) => (Some(mime.to_string()), a.wl_actions()),
            None => (None, (DndAction::empty(), DndAction::empty())),
        };
        inc.offer.accept(inc.serial, mime);
        inc.offer.set_actions(actions, preferred);
    }

    /// The data of a drop on `win` arrived (None when the read failed):
    /// hand it over, then finish the drop, unless it asks and waits for
    /// `Reply::answer_drop`. A drop whose offer is gone, whose read
    /// failed, or that the window cannot take (closed, or behind a modal
    /// child) is cancelled, and the window hears `DragLeave` instead.
    fn dropped(
        &mut self,
        win: WinId,
        offer: WlDataOffer,
        (x, y): (i32, i32),
        source: Actions,
        data: Option<Dropped>,
        action: DragAction,
    ) {
        let ev = data
            .filter(|_| offer.is_alive())
            .map(|data| Event::DragDrop {
                x,
                y,
                data,
                action,
                source,
            });
        let Some(ev) = ev.filter(|ev| self.slot_of_id(win) && !self.gated(win, ev)) else {
            offer.destroy();
            self.send(win, Event::DragLeave);
            return;
        };
        if action == DragAction::Ask {
            let asked = Asked {
                win,
                offer: offer.clone(),
                source,
            };
            if let Some(old) = self.asked_drop.replace(asked) {
                self.cancel_asked(old);
            }
            let timed = offer.clone();
            let _ = self.loop_handle.insert_source(
                Timer::from_duration(ASK_TIMEOUT),
                move |_, _, host| {
                    if host.asked_drop.as_ref().is_some_and(|a| a.offer == timed)
                        && let Some(a) = host.asked_drop.take()
                    {
                        host.cancel_asked(a);
                    }
                    TimeoutAction::Drop
                },
            );
        }
        self.send(win, ev);
        if action != DragAction::Ask {
            offer.finish();
            offer.destroy();
        }
    }

    /// Settle the `Ask` drop with the user's choice, or cancel it: an
    /// offer destroyed unfinished cancels the drag at its source. A
    /// choice the source does not allow would leave the compositor no
    /// action to finish with (a protocol error), so it cancels too, and
    /// so does an offer already gone; the window hears `DropCancelled`.
    fn answer_drop(&mut self, action: Option<DragAction>) {
        let Some(asked) = self.asked_drop.take() else {
            return;
        };
        match action {
            Some(a @ (DragAction::Copy | DragAction::Move))
                if asked.source.has(a) && asked.offer.is_alive() =>
            {
                asked.offer.set_actions(a.to_wl(), a.to_wl());
                asked.offer.finish();
                asked.offer.destroy();
            }
            None | Some(DragAction::Ask) => asked.offer.destroy(),
            Some(_) => self.cancel_asked(asked),
        }
    }

    fn cancel_asked(&mut self, asked: Asked) {
        asked.offer.destroy();
        self.send(asked.win, Event::DropCancelled);
    }

    fn peek_files(&mut self, offer: WlDataOffer, mime: &'static str) {
        let pipe = match smithay_client_toolkit::data_device_manager::data_offer::receive(
            &offer,
            mime.to_string(),
        ) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("drag: no pipe for {mime}: {e}");
                return;
            }
        };
        read_capped(
            &self.loop_handle,
            pipe,
            reprisefs::pipe::PASTE_TIMEOUT,
            move |host: &mut Host, buf| {
                let Some(buf) = buf else {
                    return;
                };
                if mime == dnd::FILE_TRANSFER {
                    host.retrieve(buf.bytes(), move |host, paths| match paths {
                        Some(paths) => host.peeked(&offer, paths),
                        // No files through the portal (a source that sends
                        // its key only at the drop): the URI list.
                        None => {
                            let uris = host.drag_in.as_ref().is_some_and(|i| {
                                i.offer == offer && i.mimes.iter().any(|m| m == URI_LIST)
                            });
                            if uris {
                                host.peek_files(offer, URI_LIST);
                            }
                        }
                    });
                } else {
                    host.peeked(
                        &offer,
                        reprisefs::fileclip::decode(URI_LIST, buf.bytes()).paths,
                    );
                }
            },
        );
    }

    fn peeked(&mut self, offer: &WlDataOffer, paths: Vec<PathBuf>) {
        let Some(inc) = self.drag_in.as_mut().filter(|i| &i.offer == offer) else {
            return;
        };
        inc.files = Some(!paths.is_empty());
        let win = inc.win;
        self.send(win, Event::DragFiles { paths });
    }

    fn retrieve(
        &self,
        bytes: &[u8],
        done: impl FnOnce(&mut Host, Option<Vec<PathBuf>>) + Send + 'static,
    ) {
        let tx = self.portal_done.clone();
        reprisebus::filetransfer::retrieve_key(bytes, move |paths| {
            let _ = tx.send(Box::new(move |host| done(host, paths)));
        });
    }

    fn slot_of_id(&self, win: WinId) -> bool {
        self.slots.iter().any(|s| s.id == win)
    }
}

impl DataDeviceHandler for Host {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataDevice,
        x: f64,
        y: f64,
        surface: &wl_surface::WlSurface,
    ) {
        let Some(offer) = self
            .data_device
            .as_ref()
            .and_then(|d| d.data().drag_offer())
        else {
            return;
        };
        offer.accept_mime_type(offer.serial, None);
        // A v1 or v2 offer has no actions and no finish: the host takes
        // only drops it can settle, so such a drag is refused.
        if offer.inner().version() < 3 {
            return;
        }
        let Some(win) = self.slot_of(surface) else {
            return;
        };
        let mimes = offer.with_mime_types(|m| m.to_vec());
        let offered = Offered::from_mimes(&mimes);
        let action = DragAction::from_wl(offer.selected_action);
        let source = Actions::from_wl(offer.source_actions);
        let (x, y) = (x as i32, y as i32);
        // One dropped elsewhere and awaiting its finish is over.
        let own = self.drag_out.as_ref().is_some_and(|o| !o.dropped);
        let portal = dnd::through_portal(&mimes, own);
        let source = source.read_as(portal);
        let peek = dnd::peek_mime(&mimes, portal);
        self.drag_in = Some(Incoming {
            win,
            offer: offer.inner().clone(),
            serial: offer.serial,
            mimes,
            pos: (x, y),
            source,
            action,
            accept: None,
            files: None,
            portal,
        });
        self.send(
            win,
            Event::DragEnter {
                x,
                y,
                offered,
                source,
                action,
                own,
            },
        );
        if let Some(mime) = peek.filter(|_| !own) {
            self.peek_files(offer.inner().clone(), mime);
        }
    }

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {
        if let Some(inc) = self.drag_in.take() {
            self.send(inc.win, Event::DragLeave);
        }
    }

    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, x: f64, y: f64) {
        let Some(inc) = self.drag_in.as_mut() else {
            return;
        };
        let (x, y) = (x as i32, y as i32);
        inc.pos = (x, y);
        let (win, action) = (inc.win, inc.action);
        self.send(win, Event::DragMotion { x, y, action });
    }

    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}

    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {
        let Some(inc) = self.drag_in.take() else {
            return;
        };
        let (Some((accept, mime)), Some(action)) = (inc.accept, inc.action) else {
            inc.offer.destroy();
            self.send(inc.win, Event::DragLeave);
            return;
        };
        let landing = Landing {
            win: inc.win,
            uris: inc.mimes.iter().any(|m| m == URI_LIST),
            offer: inc.offer,
            pos: inc.pos,
            source: inc.source,
            kind: accept.kind,
            action,
        };
        self.read_drop(landing, mime);
    }
}

struct Landing {
    win: WinId,
    offer: WlDataOffer,
    pos: (i32, i32),
    source: Actions,
    kind: DropKind,
    action: DragAction,
    uris: bool,
}

impl Host {
    fn read_drop(&mut self, d: Landing, mime: &'static str) {
        let pipe = match smithay_client_toolkit::data_device_manager::data_offer::receive(
            &d.offer,
            mime.to_string(),
        ) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("drop: no pipe for {mime}: {e}");
                d.offer.destroy();
                self.send(d.win, Event::DragLeave);
                return;
            }
        };
        read_capped(
            &self.loop_handle,
            pipe,
            reprisefs::pipe::PASTE_TIMEOUT,
            move |host: &mut Host, buf| match buf {
                Some(b) if mime == dnd::FILE_TRANSFER => {
                    host.retrieve(b.bytes(), move |host, paths| match paths {
                        Some(p) => {
                            let data = dnd::portal_drop(p, d.kind);
                            host.land(d, Some(data));
                        }
                        None if d.uris && d.offer.is_alive() => host.read_drop(d, URI_LIST),
                        None => host.land(d, None),
                    });
                }
                buf => {
                    let data = buf.map(|b| dnd::decode_drop(b.bytes(), d.kind, mime));
                    host.land(d, data);
                }
            },
        );
    }

    fn land(&mut self, d: Landing, data: Option<Dropped>) {
        self.dropped(d.win, d.offer, d.pos, d.source, data, d.action);
    }
}

impl DataOfferHandler for Host {
    /// The dragging program changed what it allows. SCTK holds the
    /// offer's lock here, so only the raw offer is touched.
    fn source_actions(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        offer: &mut DragOffer,
        actions: DndAction,
    ) {
        if let Some(inc) = self.drag_in.as_mut().filter(|i| &i.offer == offer.inner()) {
            inc.source = Actions::from_wl(actions).read_as(inc.portal);
        }
    }

    /// The compositor picked another action for the drag over us. SCTK
    /// holds the offer's lock here, so only the raw offer is touched.
    fn selected_action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        offer: &mut DragOffer,
        action: DndAction,
    ) {
        let Some(inc) = self.drag_in.as_mut().filter(|i| &i.offer == offer.inner()) else {
            return;
        };
        let action = DragAction::from_wl(action);
        if inc.action == action {
            return;
        }
        inc.action = action;
        let (win, (x, y)) = (inc.win, inc.pos);
        self.send(win, Event::DragMotion { x, y, action });
    }
}

impl DataSourceHandler for Host {
    fn accept_mime(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: Option<String>,
    ) {
    }
    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &WlDataSource,
        mime: String,
        fd: WritePipe,
    ) {
        let clipboard = self
            .copy_source
            .as_ref()
            .is_some_and(|s| s.inner() == source);
        // A drag source that is no longer ours sends nothing, never the clipboard.
        let bytes: Arc<[u8]> = if clipboard {
            if let Some(offers) = &self.copy_clip {
                match offers.iter().find(|(m, _)| *m == mime) {
                    Some((_, bytes)) => bytes.as_slice().into(),
                    None => return,
                }
            } else {
                self.copy_text.clone()
            }
        } else if let Some(out) = self.drag_of(source) {
            // Each request is its own transfer: a target may read the key
            // while the drag passes over it and again at the drop.
            if mime == dnd::FILE_TRANSFER {
                let fd: std::os::fd::OwnedFd = fd.into();
                out.transfers.send_key(out.paths.clone(), move |key| {
                    reprisefs::pipe::write_detached(fd, key.into())
                });
                return;
            }
            match dnd::encode_files(&out.paths, &mime) {
                Some(b) => b.into(),
                None => return,
            }
        } else {
            return;
        };
        // The receiver owns the pipe; the write must not stall the loop.
        reprisefs::pipe::write_detached(fd.into(), bytes);
    }
    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        if self.drag_of(source).is_some() {
            if let Some(out) = self.drag_out.take() {
                self.drag_ended(out, false);
            }
        } else if self
            .copy_source
            .as_ref()
            .is_some_and(|s| s.inner() == source)
        {
            self.copy_source = None;
        }
    }
    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        if let Some(out) = self.drag_of(source) {
            out.dropped = true;
        }
    }
    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        if self.drag_of(source).is_some()
            && let Some(out) = self.drag_out.take()
        {
            self.drag_ended(out, true);
        }
    }
    fn action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &WlDataSource,
        action: DndAction,
    ) {
        if let Some(out) = self.drag_of(source) {
            out.action = DragAction::from_wl(action);
        }
    }
}

impl ActivationHandler for Host {
    /// The window a token was asked for ([`Host::raise`]).
    type RequestUdata = Option<WinId>;
    fn new_token(&mut self, token: String, data: &RequestData<Option<WinId>>) {
        let surface = data
            .udata
            .and_then(|id| self.slots.iter().find(|s| s.id == id))
            .map(|s| s.window.wl_surface());
        if let (Some(surface), Some(activation)) = (surface, &self.activation) {
            activation.activate::<Host>(surface, token);
        }
    }
}

struct NoEvents;

fn send_tag(
    manager: Option<&xdg_toplevel_tag_manager_v1::XdgToplevelTagManagerV1>,
    window: &Window,
    tag: &str,
) {
    if let Some(m) = manager {
        m.set_toplevel_tag(window.xdg_toplevel(), tag.into());
    }
}

/// A main window's owner in another program: the imported handle and,
/// when modal, the xdg-dialog object. Held only to stay alive; dropping
/// it destroys both, which undoes the parent and the modality.
struct Foreign {
    imported: Imported,
    dialog: Option<xdg_dialog_v1::XdgDialogV1>,
}

enum Imported {
    V2(zxdg_imported_v2::ZxdgImportedV2),
    V1(zxdg_imported_v1::ZxdgImportedV1),
}

impl Drop for Foreign {
    fn drop(&mut self) {
        if let Some(d) = self.dialog.take() {
            d.destroy();
        }
        match &self.imported {
            Imported::V2(i) => i.destroy(),
            Imported::V1(i) => i.destroy(),
        }
    }
}

struct ForeignParent;

impl Dispatch2<zxdg_imported_v2::ZxdgImportedV2, Host> for ForeignParent {
    fn event(
        &self,
        _: &mut Host,
        _: &zxdg_imported_v2::ZxdgImportedV2,
        event: zxdg_imported_v2::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
        if let zxdg_imported_v2::Event::Destroyed = event {
            log::info!("the foreign parent is gone; the window stays, ownerless");
        }
    }
}

impl Dispatch2<zxdg_imported_v1::ZxdgImportedV1, Host> for ForeignParent {
    fn event(
        &self,
        _: &mut Host,
        _: &zxdg_imported_v1::ZxdgImportedV1,
        event: zxdg_imported_v1::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
        if let zxdg_imported_v1::Event::Destroyed = event {
            log::info!("the foreign parent is gone; the window stays, ownerless");
        }
    }
}

macro_rules! no_events {
    ($($iface:ident::$ty:ident),* $(,)?) => {$(
        impl Dispatch2<$iface::$ty, Host> for NoEvents {
            fn event(
                &self,
                _: &mut Host,
                _: &$iface::$ty,
                _: $iface::Event,
                _: &Connection,
                _: &QueueHandle<Host>,
            ) {
            }
        }
    )*};
}

no_events!(
    ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1,
    ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
    zxdg_importer_v2::ZxdgImporterV2,
    zxdg_importer_v1::ZxdgImporterV1,
    xdg_wm_dialog_v1::XdgWmDialogV1,
    xdg_dialog_v1::XdgDialogV1,
    xdg_toplevel_tag_manager_v1::XdgToplevelTagManagerV1,
    zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1,
    reprise_tabs_v1::RepriseTabsV1,
);

struct InhibitorOf(WinId);

impl Dispatch2<zwp_keyboard_shortcuts_inhibitor_v1::ZwpKeyboardShortcutsInhibitorV1, Host>
    for InhibitorOf
{
    fn event(
        &self,
        host: &mut Host,
        _: &zwp_keyboard_shortcuts_inhibitor_v1::ZwpKeyboardShortcutsInhibitorV1,
        event: zwp_keyboard_shortcuts_inhibitor_v1::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
        if let zwp_keyboard_shortcuts_inhibitor_v1::Event::Inactive = event {
            host.send(self.0, Event::ShortcutsInactive);
        }
    }
}

impl Dispatch2<wp_viewporter::WpViewporter, Host> for NoEvents {
    fn event(
        &self,
        _: &mut Host,
        _: &wp_viewporter::WpViewporter,
        _: wp_viewporter::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
    }
}

impl Dispatch2<wp_viewport::WpViewport, Host> for NoEvents {
    fn event(
        &self,
        _: &mut Host,
        _: &wp_viewport::WpViewport,
        _: wp_viewport::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
    }
}

impl Dispatch2<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, Host> for NoEvents {
    fn event(
        &self,
        _: &mut Host,
        _: &wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
        _: wp_fractional_scale_manager_v1::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
    }
}

struct SurfaceOf(wl_surface::WlSurface);

impl Dispatch2<wp_fractional_scale_v1::WpFractionalScaleV1, Host> for SurfaceOf {
    fn event(
        &self,
        host: &mut Host,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            host.rescale(&self.0, Some(scale), None);
        }
    }
}

impl Dispatch2<zwp_text_input_manager_v3::ZwpTextInputManagerV3, Host> for NoEvents {
    fn event(
        &self,
        _: &mut Host,
        _: &zwp_text_input_manager_v3::ZwpTextInputManagerV3,
        _: zwp_text_input_manager_v3::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
    }
}

struct TextInputOf;

impl Dispatch2<zwp_text_input_v3::ZwpTextInputV3, Host> for TextInputOf {
    fn event(
        &self,
        host: &mut Host,
        _: &zwp_text_input_v3::ZwpTextInputV3,
        event: zwp_text_input_v3::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
        host.text_input_event(event);
    }
}

impl Host {
    fn mark_dirty(&mut self, id: WinId) {
        if let Some(s) = self.slot_mut(id) {
            s.dirty = true;
        }
    }

    /// Redraw flyout `id`, which just gained or lost the keyboard, so its
    /// default button follows. A toplevel redraws on the configure that
    /// changes its `activated`.
    fn redraw_if_active_shows(&mut self, id: WinId) {
        let flyout = self
            .slots
            .iter()
            .any(|s| s.id == id && matches!(s.window, Shell::Layer(_)));
        if flyout {
            self.mark_dirty(id);
        }
    }

    fn text_input_event(&mut self, event: zwp_text_input_v3::Event) {
        use zwp_text_input_v3::Event as E;
        let entered = match &event {
            E::Enter { surface } => self.slot_of(surface),
            _ => None,
        };
        let Some(t) = self.text_input.as_mut() else {
            return;
        };
        match event {
            E::Enter { .. } => {
                if let Some(id) = entered {
                    t.enter(id);
                    self.mark_dirty(id);
                }
            }
            E::Leave { .. } => {
                if let Some((id, true)) = t.leave() {
                    self.mark_dirty(id);
                }
            }
            E::PreeditString {
                text,
                cursor_begin,
                cursor_end,
            } => t.preedit_string(text, cursor_begin, cursor_end),
            E::CommitString { text } => t.commit_string(text),
            E::DeleteSurroundingText {
                before_length,
                after_length,
            } => t.delete_surrounding(before_length, after_length),
            E::Done { .. } => {
                let Some(win) = t.entered else {
                    return;
                };
                let keys = t.done();
                self.ctx.caret_on = true;
                for key in keys {
                    let utf8 = match &key {
                        EditKey::Insert(s) => Some(s.clone()),
                        _ => None,
                    };
                    let input = KeyInput {
                        sym: 0,
                        utf8,
                        mods: Mods::default(),
                        edit: Some(key),
                    };
                    self.send(win, Event::Key(input));
                }
                self.mark_dirty(win);
            }
            _ => {}
        }
    }
}

delegate_dispatch2!(Host);
delegate_registry!(Host);

impl ProvidesRegistryState for Host {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState,];
}

/// Ids are fixed strings, so a program has at most one window per id.
pub type WinId = &'static str;

pub const MAIN: WinId = "main";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Child {
    pub id: WinId,
    pub title: String,
    pub size: (i32, i32),
    /// Fixed size: min and max are pinned to `size`, so the compositor
    /// gives it no resize band.
    pub fixed: bool,
    pub modal: bool,
}

/// A panel popped up over everything at a point on screen: the tray's
/// flyout. `at` is in global layout coordinates (what an SNI `Activate`
/// carries). The host gives it an overlay layer surface covering the
/// whole output that holds `at`, taskbar included, with the keyboard; the
/// program draws its panel into that canvas (`Ctx::origin` is the
/// output's top-left) and leaves the rest transparent, so a press there
/// is a click outside. An `at` on no output, such as
/// [`Flyout::UNDER_POINTER`], leaves the output to the compositor, which
/// takes the one under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flyout {
    pub id: WinId,
    pub at: (i32, i32),
    /// What a screen reader calls the flyout's window.
    pub title: &'static str,
}

impl Flyout {
    pub const UNDER_POINTER: (i32, i32) = (i32::MIN, i32::MIN);
}

pub(crate) fn reconcile(have: &[WinId], want: &[&Child]) -> (Vec<WinId>, Vec<usize>) {
    let drop = have
        .iter()
        .filter(|id| !want.iter().any(|c| c.id == **id))
        .copied()
        .collect();
    let make = want
        .iter()
        .enumerate()
        .filter(|(_, c)| !have.contains(&c.id))
        .map(|(i, _)| i)
        .collect();
    (drop, make)
}

/// The text types a paste reads, best first; `STRING` is X11's Latin-1 one.
const PASTE_MIMES: [&str; 5] = [TEXT_MIME, "text/plain", "UTF8_STRING", "TEXT", "STRING"];

/// `buf` read as type `mime` of [`PASTE_MIMES`].
fn pasted_text(mime: &str, buf: PasteBuf) -> String {
    if mime == "STRING" {
        buf.into_latin1_text()
    } else {
        buf.into_text()
    }
}

/// The type a paste reads out of a selection offering `types`, if it offers text.
fn text_mime(types: &[String]) -> Option<&'static str> {
    PASTE_MIMES
        .into_iter()
        .find(|m| types.iter().any(|t| t == m))
}

thread_local! {
    /// Set on the event loop's thread before each `App::event`.
    static CLIPBOARD_TEXT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Set alongside `CLIPBOARD_TEXT`.
    static CLIPBOARD_FILES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether the clipboard holds text a paste ([`Reply::paste`]) would read, as of the event being
/// handled; false before a window first had the keyboard.
pub fn clipboard_has_text() -> bool {
    CLIPBOARD_TEXT.get()
}

/// Stands in for the compositor's offer in a program's tests, on the calling thread.
#[doc(hidden)]
pub fn fake_clipboard_text(on: bool) {
    CLIPBOARD_TEXT.set(on);
}

/// Whether the clipboard holds files a [`Reply::paste_files`] would read, as of the event being
/// handled; false before a window first had the keyboard.
pub fn clipboard_has_files() -> bool {
    CLIPBOARD_FILES.get()
}

/// Stands in for the compositor's offer in a program's tests, on the calling thread.
#[doc(hidden)]
pub fn fake_clipboard_files(on: bool) {
    CLIPBOARD_FILES.set(on);
}

static TAB_GROUPS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the compositor puts a window into another's caption tab group ([`App::tab_with`]),
/// known once [`run`] connected, before [`App::start`].
pub fn tab_groups() -> bool {
    TAB_GROUPS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Where a child whose program names `parent` opens, given whether that parent's window is
/// mapped (`None`: it has no window).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Placement {
    Wait,
    Under(WinId),
    Alone,
}

pub(crate) fn placement(parent: Option<WinId>, mapped: Option<bool>) -> Placement {
    match (parent, mapped) {
        (Some(_), Some(false)) => Placement::Wait,
        (Some(p), Some(true)) => Placement::Under(p),
        _ => Placement::Alone,
    }
}

/// The toplevel `win` belongs to: the window up its chain of [`App::parent`]s that has none,
/// or `MAIN`.
pub(crate) fn root_of(win: WinId, parent: impl Fn(WinId) -> Option<WinId>) -> WinId {
    let mut w = win;
    // A chain is a dialog over a dialog at most; the bound stops a loop a program made.
    for _ in 0..8 {
        if w == MAIN {
            break;
        }
        match parent(w) {
            Some(p) if p != w => w = p,
            _ => break,
        }
    }
    w
}

/// The modal child among `slots` that gates `win`: the newest with the same toplevel.
pub(crate) fn modal_over(
    win: WinId,
    slots: impl Iterator<Item = (WinId, bool)>,
    parent: impl Fn(WinId) -> Option<WinId>,
) -> Option<WinId> {
    let root = root_of(win, &parent);
    top_modal(slots.filter(|&(id, _)| root_of(id, &parent) == root))
}

/// The id `window-<n>`, one `'static` str per `n` for the life of the process: for a program
/// that opens windows of its own as it runs ([`App::parent`]).
pub fn window_id(n: usize) -> WinId {
    use std::sync::Mutex;
    static IDS: Mutex<Vec<WinId>> = Mutex::new(Vec::new());
    let mut ids = IDS.lock().unwrap_or_else(|e| e.into_inner());
    while ids.len() <= n {
        let next = format!("window-{}", ids.len());
        ids.push(Box::leak(next.into_boxed_str()));
    }
    ids[n]
}

/// The newest, since each child maps over the ones before it.
pub(crate) fn top_modal(slots: impl Iterator<Item = (WinId, bool)>) -> Option<WinId> {
    slots.filter(|&(_, modal)| modal).map(|(id, _)| id).last()
}

/// Whether `ev` on `win` is dropped because modal child `modal` is up:
/// keys and pointer go to that child alone. A flyout is exempt: it sits
/// over the whole output, the modal child included, so gating it would
/// leave nothing on screen that takes input.
pub(crate) fn input_blocked(ev: &Event, modal: Option<WinId>, win: WinId, flyout: bool) -> bool {
    let input = matches!(
        ev,
        Event::Key(_)
            | Event::KeyUp(_)
            | Event::Pointer(_)
            | Event::DragEnter { .. }
            | Event::DragMotion { .. }
            | Event::DragFiles { .. }
            | Event::DragDrop { .. }
    );
    input && !flyout && modal.is_some_and(|m| m != win)
}

/// The keyboard's window, else the pointer's (a hover tip ticks on the blink), else none.
pub(crate) fn blink_target(focus: Option<WinId>, pointer_in: Option<WinId>) -> Option<WinId> {
    focus.or(pointer_in)
}

pub(crate) fn flyout_wanted(
    want: Option<Flyout>,
    have: Option<WinId>,
) -> (Option<WinId>, Option<Flyout>) {
    match (want, have) {
        (Some(f), Some(id)) if f.id == id => (None, None),
        (want, have) => (have, want),
    }
}

pub(crate) fn output_containing(outputs: &[(i32, i32, i32, i32)], at: (i32, i32)) -> Option<usize> {
    outputs
        .iter()
        .position(|&(x, y, w, h)| Rect::new(x, y, w, h).contains(at.0, at.1))
}

pub(crate) fn main_wanted(hidden: bool, have: bool) -> Option<bool> {
    match (hidden, have) {
        (false, false) => Some(true),
        (true, true) => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(t: &[&str]) -> Vec<String> {
        t.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_selection_offering_any_text_type_reads_its_best_one() {
        let browser = types(&["text/html", "text/plain", TEXT_MIME]);
        assert_eq!(text_mime(&browser), Some(TEXT_MIME));
        let x11 = types(&["TARGETS", "STRING", "UTF8_STRING"]);
        assert_eq!(text_mime(&x11), Some("UTF8_STRING"));
        assert_eq!(text_mime(&types(&["TEXT"])), Some("TEXT"));
        assert_eq!(text_mime(&types(&["STRING"])), Some("STRING"));
    }

    #[test]
    fn a_string_paste_reads_as_latin1_and_the_others_as_utf8() {
        let buf = |b: &[u8]| {
            let mut buf = PasteBuf::default();
            assert!(buf.push(b));
            buf
        };
        assert_eq!(pasted_text("STRING", buf(b"caf\xe9")), "caf\u{e9}");
        assert_eq!(
            pasted_text("UTF8_STRING", buf("caf\u{e9}".as_bytes())),
            "caf\u{e9}"
        );
        assert_eq!(
            pasted_text("TEXT", buf("caf\u{e9}".as_bytes())),
            "caf\u{e9}"
        );
    }

    #[test]
    fn an_image_or_files_on_the_clipboard_is_no_text() {
        assert_eq!(text_mime(&types(&["image/png", "image/bmp"])), None);
        assert_eq!(
            text_mime(&types(&["text/uri-list", "x-special/gnome-copied-files"])),
            None
        );
        assert_eq!(text_mime(&types(&["text/html"])), None);
        assert_eq!(text_mime(&[]), None);
    }

    #[test]
    fn reply_merge_keeps_primary() {
        let first = Reply {
            copy_primary: Some("old".into()),
            paste_primary: false,
            ..Default::default()
        };
        let then = Reply {
            copy_primary: Some("new".into()),
            paste_primary: true,
            ..Default::default()
        };
        let merged = first.clone().and(then);
        assert_eq!(merged.copy_primary.as_deref(), Some("old"));
        assert!(merged.paste_primary);
        let filled = Reply::default().and(Reply {
            copy_primary: Some("new".into()),
            ..Default::default()
        });
        assert_eq!(filled.copy_primary.as_deref(), Some("new"));
    }

    #[test]
    fn damage_rects_scale_to_buffer() {
        let rects = [Rect::new(1, 2, 3, 4), Rect::new(0, 0, 10, 5)];
        assert_eq!(buffer_damage(&rects, 2), vec![(2, 4, 6, 8), (0, 0, 20, 10)]);
    }

    #[test]
    fn only_a_kept_canvas_damages_part_of_the_buffer() {
        let rects = Some(vec![Rect::new(1, 2, 3, 4)]);
        let whole = vec![(0, 0, 40, 20)];
        let damage = |rects, fresh, keeps| surface_damage(rects, fresh, keeps, 2, (40, 20));
        assert_eq!(damage(rects.clone(), true, true), vec![(2, 4, 6, 8)]);
        assert_eq!(
            damage(rects.clone(), true, false),
            whole,
            "a canvas cleared for the draw damages the whole buffer"
        );
        assert_eq!(damage(rects, false, true), whole, "so does a new canvas");
        assert_eq!(damage(None, true, true), whole);
    }

    #[test]
    fn a_spawned_program_is_handed_over_unreaped_as_this_process_s_child() {
        let (tx, rx) = std::sync::mpsc::channel();
        spawn_then(&["true".into()], None, None, move |pid| {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
            let ppid = stat
                .rsplit(") ")
                .next()
                .and_then(|rest| rest.split(' ').nth(1));
            tx.send(ppid.map(str::to_owned)).unwrap();
        })
        .unwrap();
        let ppid = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(ppid, Some(std::process::id().to_string()));
    }

    struct Kept {
        kept: (i32, i32),
        min: Option<(i32, i32)>,
        fixed: bool,
    }

    impl App for Kept {
        fn title(&self) -> String {
            String::new()
        }
        fn app_id(&self) -> String {
            String::new()
        }
        fn initial_size(&self) -> (i32, i32) {
            self.kept
        }
        fn fixed(&self) -> bool {
            self.fixed
        }
        fn min_size(&self, _: WinId) -> Option<(i32, i32)> {
            self.min
        }
        fn event(&mut self, _: WinId, _: Event, _: &mut Ctx) -> Reply {
            Reply::default()
        }
        fn draw(&mut self, _: WinId, _: &mut Canvas, _: &Ctx) {}
    }

    #[test]
    fn a_kept_size_under_the_minimum_opens_at_the_minimum() {
        let app = |kept, min, fixed| Kept { kept, min, fixed };
        let min = Some((544, 528));
        assert_eq!(
            opening_size(&app((760, 500), min, false)),
            ((760, 528), (544, 528))
        );
        assert_eq!(
            opening_size(&app((900, 700), min, false)),
            ((900, 700), (544, 528))
        );
        assert_eq!(
            opening_size(&app((50, 20), None, false)),
            (MIN_SIZE, MIN_SIZE)
        );
        assert_eq!(
            opening_size(&app((300, 200), min, true)),
            ((300, 200), (300, 200)),
            "a fixed window is pinned to its own size"
        );
    }

    #[test]
    fn theme_from_env_with_no_interface_is_the_default_theme() {
        assert_eq!(
            theme_from_env(None, None, None, None, None),
            Theme::default(),
            "no REPRISE_INTERFACE and no REPRISE_COLORS must give Theme::default()"
        );
    }

    #[test]
    fn theme_from_env_with_blue_takes_the_blue_style() {
        let t = theme_from_env(Some("blue"), None, None, None, None);
        let blue = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
        assert_eq!(
            t.colors, blue.colors,
            "blue interface with no REPRISE_COLORS must keep Blue's own colours"
        );
        assert!(
            matches!(t.look, reprisekit::look::Look::Styled(_)),
            "blue interface must draw through the styled look"
        );
    }

    #[test]
    fn theme_from_env_picks_the_blue_variant() {
        use reprisekit::blue::Variant;
        use reprisekit::interfaces::blue_theme;
        for v in Variant::ALL {
            let t = theme_from_env(Some("blue"), Some(v.as_str()), None, None, None);
            assert_eq!(t.look, blue_theme(v).look, "{v:?}");
            assert_eq!(t.colors, blue_theme(v).colors, "{v:?}");
        }
        for unknown in [None, Some("green"), Some("Silver")] {
            assert_eq!(
                theme_from_env(Some("blue"), unknown, None, None, None).look,
                blue_theme(Variant::Blue).look,
                "{unknown:?} is navy"
            );
        }
        assert_eq!(
            theme_from_env(Some("classic"), Some("olive"), None, None, None),
            Theme::default(),
            "the variant means nothing outside Blue"
        );
    }

    #[test]
    fn the_glass_mode_picks_dark_glass_and_anything_else_is_light() {
        let dark = theme_from_env(Some("glass"), None, Some("dark"), None, None);
        let light = theme_from_env(Some("glass"), None, None, None, None);
        assert_ne!(dark.look, light.look, "dark Glass is another look");
        assert_eq!(dark.look.style().unwrap().name, "glass-dark");
        for odd in [Some("Dark"), Some(""), Some("dim")] {
            assert_eq!(
                theme_from_env(Some("glass"), None, odd, None, None).look,
                light.look,
                "{odd:?} is light"
            );
        }
        assert_eq!(
            theme_from_env(Some("classic"), None, Some("dark"), None, None).look,
            theme_from_env(Some("classic"), None, None, None, None).look,
            "Classic ignores the glass mode"
        );
    }

    #[test]
    fn smoked_glass_inks_labels_on_its_glass_white_in_either_mode() {
        for mode in ["light", "dark"] {
            let theme = theme_from_env(Some("glass"), None, Some(mode), Some("smoked"), None);
            assert_eq!(
                reprisekit::look::ink_on_glass(&theme),
                Some(reprisekit::theme::Color::rgb(0xFF, 0xFF, 0xFF)),
                "{mode}"
            );
        }
        let frosted = theme_from_env(Some("glass"), None, Some("light"), None, None);
        assert_eq!(
            reprisekit::look::ink_on_glass(&frosted),
            Some(reprisekit::theme::Color::rgb(0, 0, 0)),
            "light Frosted inks black"
        );
    }

    #[test]
    fn a_glass_material_change_through_the_portal_relooks() {
        let mut look = Look {
            interface: Some("glass".into()),
            ..Look::default()
        };
        let ink = |l: &Look| reprisekit::look::ink_on_glass(&l.theme());
        let before = ink(&look);
        assert!(look.set(reprisebus::appearance::GLASS_MATERIAL, "smoked".into()));
        assert_ne!(ink(&look), before, "the ink on glass follows");
    }

    #[test]
    fn a_glass_mode_change_through_the_portal_relooks() {
        let mut look = Look {
            interface: Some("glass".into()),
            ..Look::default()
        };
        let before = look.theme().look;
        assert!(
            look.set(reprisebus::appearance::GLASS_MODE, "dark".into()),
            "the change is seen"
        );
        assert_ne!(look.theme().look, before, "and the look moves");
        assert!(
            !look.set(reprisebus::appearance::GLASS_MODE, "dark".into()),
            "the same value again is no change"
        );
    }

    #[test]
    fn a_new_blue_variant_rebuilds_the_theme() {
        use reprisebus::appearance as a;
        let mut look = Look {
            interface: Some("blue".into()),
            ..Look::default()
        };
        assert!(look.set(a::BLUE_VARIANT, "silver".into()));
        assert!(!look.set(a::BLUE_VARIANT, "silver".into()));
        assert_eq!(
            look.theme().look,
            reprisekit::interfaces::blue_theme(reprisekit::blue::Variant::Silver).look
        );
    }

    #[test]
    fn a_new_second_dialog_font_reaches_the_themes_fonts() {
        use reprisebus::appearance as a;
        let mut look = Look::default();
        assert!(look.set(a::SHELL_DLG, "Tahoma".into()));
        assert!(!look.set(a::SHELL_DLG, "Tahoma".into()));
        assert_eq!(look.theme().fonts.shell_dlg.as_deref(), Some("Tahoma"));
        assert!(look.set(a::SHELL_DLG, String::new()), "unset is a change");
        assert_eq!(look.theme().fonts.shell_dlg, None);
    }

    #[test]
    fn the_same_fonts_share_one_renderer_and_other_fonts_load_another() {
        let fonts = reprisekit::theme::FontConfig::default();
        assert!(Rc::ptr_eq(&text_for(&fonts), &text_for(&fonts)));
        let other = reprisekit::theme::FontConfig {
            family: vec!["DejaVu Sans".into()],
            ..fonts.clone()
        };
        let changed = text_for(&other);
        assert!(Rc::ptr_eq(&changed, &text_for(&other)));
        assert!(!Rc::ptr_eq(&changed, &text_for(&fonts)));
    }

    #[test]
    fn the_configured_font_family_reaches_the_themes_fonts() {
        use reprisebus::appearance as a;
        let mut look = Look::default();
        let default = look.theme().fonts.family;
        assert!(look.set(a::FONT_FAMILY, "DejaVu Sans, Noto Sans".into()));
        assert!(!look.set(a::FONT_FAMILY, "DejaVu Sans,Noto Sans".into()));
        assert_eq!(look.theme().fonts.family, ["DejaVu Sans", "Noto Sans"]);
        assert!(look.set(a::FONT_FAMILY, String::new()), "unset is a change");
        assert_eq!(look.theme().fonts.family, default);
    }

    #[test]
    fn the_configured_antialias_reaches_the_themes_fonts() {
        use reprisebus::appearance as a;
        let mut look = Look::default();
        assert!(!look.theme().fonts.antialias);
        assert!(look.set(a::FONT_ANTIALIAS, "true".into()));
        assert!(!look.set(a::FONT_ANTIALIAS, "true".into()));
        assert!(!look.set(a::FONT_ANTIALIAS, "smooth".into()));
        assert!(look.theme().fonts.antialias);
        assert!(look.set(a::FONT_ANTIALIAS, "false".into()));
        assert!(!look.theme().fonts.antialias);
    }

    #[test]
    fn the_configured_double_click_time_reaches_the_themes_metrics() {
        use reprisebus::appearance as a;
        let mut look = Look::default();
        assert_eq!(look.theme().metrics.double_click_ms, 500);
        assert!(look.set(a::DOUBLE_CLICK_MS, "900".into()));
        assert!(!look.set(a::DOUBLE_CLICK_MS, "900".into()));
        assert!(!look.set(a::DOUBLE_CLICK_MS, "slow".into()));
        assert_eq!(look.theme().metrics.double_click_ms, 900);
    }

    #[test]
    fn a_new_colour_scheme_rebuilds_the_theme_over_the_same_interface() {
        use reprisebus::appearance as a;
        let tokyo = reprisekit::schemes::scheme("Tokyo Night").unwrap();
        let mut look = Look {
            interface: Some("classic".into()),
            ..Look::default()
        };
        assert!(
            look.set(a::COLORS, tokyo.to_env()),
            "a first scheme is a change"
        );
        assert!(
            !look.set(a::COLORS, tokyo.to_env()),
            "the same scheme again is none"
        );
        assert!(!look.set("font", "Tahoma".into()), "an unknown key is none");
        assert_eq!(
            look.theme().colors,
            tokyo,
            "the theme takes Tokyo Night's colours"
        );
        assert!(look.set(a::ICON_THEME, "Papirus".into()));
        assert_eq!(look.icon_theme, "Papirus");
    }

    #[test]
    fn theme_from_env_applies_colors_over_the_blue_style() {
        let t = theme_from_env(Some("blue"), None, None, None, Some("window_text=#010203"));
        let blue = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
        assert_eq!(
            t.colors.window_text,
            reprisekit::theme::Color::rgb(1, 2, 3),
            "the REPRISE_COLORS pair must win over Blue's own colour"
        );
        assert_eq!(
            t.colors.button_face, blue.colors.button_face,
            "a colour not named in REPRISE_COLORS must stay Blue's own"
        );
    }

    #[test]
    fn the_draw_scale_rounds_the_fraction_up_and_fits_max_side() {
        assert_eq!(draw_scale(Some(180), 1, (640, 480)), 2);
        assert_eq!(draw_scale(Some(150), 1, (640, 480)), 2);
        assert_eq!(draw_scale(Some(240), 1, (640, 480)), 2);
        assert_eq!(draw_scale(Some(120), 2, (640, 480)), 1, "the fraction wins");
        assert_eq!(draw_scale(None, 3, (640, 480)), 3);
        assert_eq!(draw_scale(None, 0, (640, 480)), 1);
        assert_eq!(
            draw_scale(None, 4, (MAX_SIDE / 2, 10)),
            2,
            "a side past MAX_SIDE"
        );
        assert_eq!(draw_scale(None, 4, (MAX_SIDE, 10)), 1);
    }

    /// A program at the text size has its forms theme as its own theme, the text grown once;
    /// any other program grows its 100% theme for its forms.
    #[test]
    fn forms_grow_the_text_once() {
        let theme = Theme::default();
        let mut ctx = Ctx {
            theme: theme.clone(),
            text: Rc::new(TextRenderer::load(&theme.fonts)),
            icons: IconCache::new(IconTheme::load("test", &[])),
            caret_on: true,
            size: (40, 30),
            maximized: false,
            origin: (0, 0),
            wake: Waker::noop(),
            heads: Default::default(),
            want_tick: Default::default(),
            at_pick: false,
            text_size: 150,
        };
        let grown = reprisekit::theme::TextUnits::new(17, 11);
        assert_eq!(ctx.forms().fonts.size_px, 17);
        assert_eq!(ctx.forms().text, grown);
        ctx.theme = theme.with_text_size(150);
        assert_eq!(ctx.forms().fonts.size_px, 17);
        assert_eq!(ctx.forms().text, grown);
    }

    /// A text-sized program's theme is already at 150%: its form units, and the px a form drawn
    /// at them hit-tests with, grow the 100% font once.
    #[test]
    fn a_text_sized_theme_grows_form_units_once() {
        let theme = Theme::default();
        let ctx = Ctx {
            theme: theme.with_text_size(150),
            text: Rc::new(TextRenderer::load(&theme.fonts)),
            icons: IconCache::new(IconTheme::load("test", &[])),
            caret_on: true,
            size: (40, 30),
            maximized: false,
            origin: (0, 0),
            wake: Waker::noop(),
            heads: Default::default(),
            want_tick: Default::default(),
            at_pick: false,
            text_size: 150,
        };
        assert_eq!(ctx.base_px(), 11);
        assert_eq!(ctx.form_units(), reprisekit::theme::TextUnits::new(17, 11));
        assert_eq!(ctx.form_units().px(ctx.base_px()), 17);
    }

    #[test]
    fn a_window_at_scale_2_paints_glass_parts_at_2() {
        use reprisekit::interface::Interface;
        use reprisekit::look::widgets::draw_close_box;
        use reprisekit::raster::{Canvas, Rect};
        let theme = reprisekit::interfaces::theme_for(Interface::Glass);
        let mut ctx = Ctx {
            theme: theme.clone(),
            text: Rc::new(TextRenderer::load(&theme.fonts)),
            icons: IconCache::new(IconTheme::load("test", &[])),
            caret_on: true,
            size: (40, 30),
            maximized: false,
            origin: (0, 0),
            wake: Waker::noop(),
            heads: Default::default(),
            want_tick: Default::default(),
            at_pick: false,
            text_size: 100,
        };
        let r = Rect::new(5, 4, 30, 20);
        let mut program = Canvas::with_scale(40, 30, 2);
        ctx.at_scale(2, |ctx| draw_close_box(&mut program, &ctx.theme.paint(), r));
        let two = theme.scaled(2);
        let mut compositor = Canvas::new(80, 60);
        draw_close_box(&mut compositor, &two.paint(), r.scaled(2));
        assert!(
            program.pixels == compositor.pixels,
            "the program's device pixels are the compositor's"
        );
        let mut blocks = Canvas::with_scale(40, 30, 2);
        draw_close_box(&mut blocks, &theme.paint(), r);
        assert!(
            program.pixels != blocks.pixels,
            "not the scale-1 part in 2x2 blocks"
        );
        let split = (0..30_usize).any(|y| {
            (0..40).any(|x| {
                let at = |dx: usize, dy: usize| program.pixels[(2 * y + dy) * 80 + 2 * x + dx];
                [at(1, 0), at(0, 1), at(1, 1)]
                    .iter()
                    .any(|&p| p != at(0, 0))
            })
        });
        assert!(split, "some 2x2 block of device px is not one colour");
        assert!(ctx.theme.look == theme.look, "the scale-1 look is back");
    }

    #[test]
    fn a_window_at_scale_2_paints_the_blue_status_bar_at_2() {
        use reprisekit::interface::Interface;
        use reprisekit::look::widgets::statusbar::draw_bar;
        use reprisekit::raster::{Canvas, Rect};
        let theme = reprisekit::interfaces::theme_for(Interface::Blue);
        let mut ctx = Ctx {
            theme: theme.clone(),
            text: Rc::new(TextRenderer::load(&theme.fonts)),
            icons: IconCache::new(IconTheme::load("test", &[])),
            caret_on: true,
            size: (120, 30),
            maximized: false,
            origin: (0, 0),
            wake: Waker::noop(),
            heads: Default::default(),
            want_tick: Default::default(),
            at_pick: false,
            text_size: 100,
        };
        let r = Rect::new(0, 8, 120, 22);
        let mut program = Canvas::with_scale(120, 30, 2);
        ctx.at_scale(2, |ctx| draw_bar(&mut program, &ctx.theme.paint(), r));
        let mut compositor = Canvas::new(240, 60);
        draw_bar(&mut compositor, &theme.scaled(2).paint(), r.scaled(2));
        assert!(
            program.pixels == compositor.pixels,
            "the program's device pixels are the compositor's"
        );
        let mut blocks = Canvas::with_scale(120, 30, 2);
        draw_bar(&mut blocks, &theme.paint(), r);
        assert!(
            program.pixels != blocks.pixels,
            "not the scale-1 bar in 2x2 blocks"
        );
    }

    #[test]
    fn a_modal_child_gates_input_but_not_the_flyout() {
        let key = Event::Pointer(PointerInput::Leave);
        assert!(input_blocked(&key, Some("error"), MAIN, false));
        assert!(!input_blocked(&key, Some("error"), "error", false));
        assert!(
            !input_blocked(&key, Some("error"), "flyout", true),
            "a flyout covers the modal child, so it must still take input"
        );
        assert!(!input_blocked(&Event::Resized, Some("error"), MAIN, false));
        assert!(!input_blocked(&key, None, MAIN, false));
    }

    #[test]
    fn a_file_uri_names_a_local_absolute_path_only() {
        let list = b"file:///tmp/a%20b\r\nfile://localhost/etc/x\r\nfile://LOCALHOST/y\r\n\
            file://other/etc/passwd\r\nfile://host\r\nfile:relative\r\n";
        assert_eq!(
            reprisefs::fileclip::decode(URI_LIST, list).paths,
            vec![
                PathBuf::from("/tmp/a b"),
                PathBuf::from("/etc/x"),
                PathBuf::from("/y")
            ]
        );
    }

    #[test]
    fn a_modal_child_refuses_drags_over_the_windows_behind_it() {
        let enter = Event::DragEnter {
            x: 0,
            y: 0,
            offered: Offered::default(),
            source: Actions::ALL,
            action: None,
            own: false,
        };
        let motion = Event::DragMotion {
            x: 0,
            y: 0,
            action: None,
        };
        assert!(input_blocked(&enter, Some("error"), MAIN, false));
        assert!(input_blocked(&motion, Some("error"), MAIN, false));
        let drop = Event::DragDrop {
            x: 0,
            y: 0,
            data: Dropped::Text(String::new()),
            action: DragAction::Copy,
            source: Actions::ALL,
        };
        assert!(input_blocked(&drop, Some("error"), MAIN, false));
        assert!(!input_blocked(&enter, Some("error"), "error", false));
        assert!(!input_blocked(
            &Event::DragLeave,
            Some("error"),
            MAIN,
            false
        ));
    }

    #[test]
    fn the_newest_modal_child_is_the_one_that_gates() {
        let slots = [
            (MAIN, false),
            ("open", true),
            ("find", false),
            ("error", true),
        ];
        assert_eq!(top_modal(slots.into_iter()), Some("error"));
        assert_eq!(top_modal(slots[..3].iter().copied()), Some("open"));
        assert_eq!(top_modal([(MAIN, false)].into_iter()), None);
    }

    #[test]
    fn a_modal_child_gates_only_the_windows_it_sits_over() {
        let parent = |w: WinId| match w {
            "confirm" => Some("second"),
            "second" => None,
            _ => Some(MAIN),
        };
        assert_eq!(root_of("confirm", parent), "second");
        assert_eq!(root_of("find", parent), MAIN);
        assert_eq!(root_of(MAIN, parent), MAIN);
        let slots = [(MAIN, false), ("second", false), ("confirm", true)];
        assert_eq!(modal_over(MAIN, slots.into_iter(), parent), None);
        assert_eq!(
            modal_over("second", slots.into_iter(), parent),
            Some("confirm")
        );
        assert_eq!(
            modal_over("confirm", slots.into_iter(), parent),
            Some("confirm")
        );
    }

    #[test]
    fn a_child_waits_for_its_parents_first_frame() {
        assert_eq!(placement(Some(MAIN), Some(false)), Placement::Wait);
        assert_eq!(placement(Some(MAIN), Some(true)), Placement::Under(MAIN));
        assert_eq!(
            placement(Some(MAIN), None),
            Placement::Alone,
            "a hidden main window parents nothing"
        );
        assert_eq!(placement(None, Some(false)), Placement::Alone);
    }

    #[test]
    fn a_numbered_window_id_is_one_str_per_number() {
        assert!(std::ptr::eq(window_id(3), window_id(3)));
        assert_ne!(window_id(3), window_id(4));
        assert_ne!(window_id(0), MAIN);
    }

    #[test]
    fn reply_merge_joins_the_windows_to_redraw_and_keeps_the_first_raise() {
        let first = Reply {
            redraw_wins: vec!["a"],
            raise: Some("a"),
            ..Default::default()
        };
        let then = Reply {
            redraw_wins: vec!["b"],
            raise: Some("b"),
            ..Default::default()
        };
        let merged = first.and(then);
        assert_eq!(merged.redraw_wins, ["a", "b"]);
        assert_eq!(merged.raise, Some("a"));
    }

    #[test]
    fn a_flyout_is_made_dropped_or_left() {
        let f = Flyout {
            id: "flyout",
            at: (10, 20),
            title: "Volume",
        };
        assert_eq!(flyout_wanted(Some(f), None), (None, Some(f)));
        assert_eq!(flyout_wanted(None, Some("flyout")), (Some("flyout"), None));
        assert_eq!(flyout_wanted(Some(f), Some("flyout")), (None, None));
        let g = Flyout {
            id: "other",
            at: (0, 0),
            title: "Other",
        };
        assert_eq!(
            flyout_wanted(Some(g), Some("flyout")),
            (Some("flyout"), Some(g))
        );
    }

    #[test]
    fn the_flyout_goes_on_the_output_under_its_point() {
        let outputs = [(0, 0, 1920, 1080), (1920, 0, 1280, 1024)];
        assert_eq!(output_containing(&outputs, (2000, 1000)), Some(1));
        assert_eq!(output_containing(&outputs, (5, 1079)), Some(0));
        assert_eq!(output_containing(&outputs, (5000, 5)), None);
        let left = [(-1920, -1080, 1920, 1080)];
        assert_eq!(output_containing(&left, Flyout::UNDER_POINTER), None);
    }

    #[test]
    fn a_size_the_program_picked_outlasts_a_repeated_configure() {
        let remembered = (Some(400), Some(300));
        assert_eq!(
            configured_size((200, 300), remembered, (None, None), false, false),
            ((400, 300), false)
        );
        assert_eq!(
            configured_size((296, 300), remembered, remembered, true, false),
            ((296, 300), true)
        );
        assert_eq!(
            configured_size((296, 300), remembered, remembered, true, true),
            ((400, 300), false)
        );
        assert_eq!(
            configured_size((296, 300), (Some(500), Some(300)), remembered, true, false),
            ((500, 300), false)
        );
        assert_eq!(
            configured_size((296, 300), (None, None), (None, None), true, false),
            ((296, 300), true)
        );
    }

    /// A text size change refits an open window: a fixed one to the size its program asks
    /// for; a resizable one at its program's pick to the size the program picks now, larger
    /// or smaller; one the user sized keeps its size, held to at least its new minimum.
    #[test]
    fn an_open_window_takes_what_its_program_asks() {
        assert_eq!(
            regrown((398, 424), (398, 424), Some((597, 636)), None),
            (597, 636)
        );
        assert_eq!(regrown((900, 700), (816, 792), None, None), (900, 792));
        assert_eq!(regrown((900, 900), (816, 792), None, None), (900, 900));
        assert_eq!(
            regrown((1088, 1000), (544, 528), None, Some((760, 680))),
            (760, 680),
            "a picked size shrinks back"
        );
        assert_eq!(
            regrown((760, 680), (1088, 1056), None, Some((1088, 1056))),
            (1088, 1056),
            "and grows"
        );
    }

    /// A window is at its program's pick while its size is that pick held inside the bounds;
    /// any other size is the user's or the compositor's.
    #[test]
    fn a_window_is_at_its_pick_until_sized_otherwise() {
        assert!(at_pick((1088, 1056), (1088, 1056), None));
        assert!(at_pick((1088, 1000), (1088, 1056), Some((1900, 1000))));
        assert!(!at_pick((1200, 1000), (1088, 1056), Some((1900, 1000))));
    }

    #[test]
    fn a_window_at_its_pick_stays_at_it_under_other_bounds() {
        let (pick, small, large) = ((1088, 1056), Some((1366, 700)), Some((1914, 1026)));
        assert_eq!(rebounded((1088, 700), pick, small, large), (1088, 1026));
        assert_eq!(rebounded((1088, 1026), pick, large, small), (1088, 700));
        assert_eq!(rebounded((1088, 700), pick, small, None), pick);
        assert_eq!(
            rebounded((1000, 650), pick, small, large),
            (1000, 650),
            "a size the user picked stays"
        );
    }

    /// Settings sized by the compositor at 952x528 grows to its 200% minimum under the old
    /// bounds, then hears the smaller ones the taller caption and taskbar leave.
    #[test]
    fn a_window_grown_past_new_bounds_is_held_inside_them() {
        let (pick, was, now) = ((760, 680), Some((1912, 1016)), Some((1912, 990)));
        assert_eq!(rebounded((1088, 1016), pick, was, now), (1088, 990));
    }

    /// A window that picks its own size stays inside the compositor's bounds: a Settings sheet
    /// grown past a 1080-line screen opens as tall as the work area. A side the compositor asks
    /// for (maximized, fullscreen) is its own.
    #[test]
    fn a_size_the_window_picks_stays_inside_the_bounds() {
        let bounds = Some((1914, 1026));
        assert_eq!(held_size((1200, 1300), (None, None), bounds), (1200, 1026));
        assert_eq!(held_size((2100, 900), (None, None), bounds), (1914, 900));
        assert_eq!(
            held_size((1920, 1080), (Some(1920), Some(1080)), bounds),
            (1920, 1080)
        );
        assert_eq!(
            held_size((1200, 1300), (Some(1200), None), bounds),
            (1200, 1026)
        );
        assert_eq!(held_size((1200, 1300), (None, None), None), (1200, 1300));
    }

    #[test]
    fn pixels_land_in_shm_byte_order() {
        let pixels = [0xAABB_CCDDu32, 0x0102_0304];
        let mut target = [0u8; 8];
        copy_pixels(&mut target, &pixels);
        assert_eq!(target, [0xDD, 0xCC, 0xBB, 0xAA, 0x04, 0x03, 0x02, 0x01]);
        let mut short = [0u8; 4];
        copy_pixels(&mut short, &pixels);
        assert_eq!(short, [0xDD, 0xCC, 0xBB, 0xAA]);
    }

    fn child(id: WinId) -> Child {
        Child {
            id,
            title: id.to_string(),
            size: (300, 150),
            fixed: true,
            modal: false,
        }
    }

    #[test]
    fn a_paste_writer_that_never_writes_is_dropped_at_the_timeout() {
        use smithay_client_toolkit::reexports::calloop::EventLoop;
        use std::io::{ErrorKind, Write};

        let mut event_loop: EventLoop<Option<bool>> = EventLoop::try_new().expect("event loop");
        let (reader, mut writer) = std::io::pipe().expect("pipe");
        read_capped(
            &event_loop.handle(),
            ReadPipe::from(std::os::fd::OwnedFd::from(reader)),
            Duration::from_millis(30),
            |got: &mut Option<bool>, buf: Option<PasteBuf>| *got = Some(buf.is_some()),
        );
        let mut got = None;
        for _ in 0..10 {
            event_loop
                .dispatch(Duration::from_millis(20), &mut got)
                .expect("dispatch");
        }
        let err = writer.write(b"x").expect_err("the read end is closed");
        assert_eq!(err.kind(), ErrorKind::BrokenPipe);
        assert_eq!(got, Some(false), "a timed-out read reports no bytes");
    }

    #[test]
    fn the_blink_goes_to_the_keyboard_then_the_pointer_then_nowhere() {
        assert_eq!(blink_target(Some("find"), Some(MAIN)), Some("find"));
        assert_eq!(blink_target(None, Some(MAIN)), Some(MAIN));
        assert_eq!(blink_target(None, None), None);
    }

    #[test]
    fn frames_alternate_between_two_buffers() {
        assert_eq!(next_buffer([None, None], 0), (0, true));
        assert_eq!(next_buffer([Some(false), None], 0), (1, true));
        assert_eq!(next_buffer([Some(true), Some(false)], 1), (0, false));
        assert_eq!(next_buffer([Some(false), Some(true)], 0), (1, false));
        assert_eq!(next_buffer([Some(false), Some(false)], 0), (1, true));
    }

    #[test]
    fn a_window_side_is_clamped_between_1_and_16384() {
        assert_eq!(sane_side(i64::from(u32::MAX)), 16384);
        assert_eq!(sane_side(i64::from(1u32 << 31)), 16384);
        assert_eq!(sane_side(-300), 1);
        assert_eq!(sane_side(0), 1);
        assert_eq!(sane_side(640), 640);
    }

    #[test]
    fn nothing_open_and_nothing_wanted_is_a_no_op() {
        let (drop, make) = reconcile(&[], &[]);
        assert!(drop.is_empty() && make.is_empty());
    }

    #[test]
    fn a_new_child_is_created() {
        let want = [child("find")];
        let (drop, make) = reconcile(&[], &want.iter().collect::<Vec<_>>());
        assert!(drop.is_empty());
        assert_eq!(make, vec![0]);
    }

    #[test]
    fn a_child_that_went_away_is_destroyed() {
        let (drop, make) = reconcile(&["find"], &[]);
        assert_eq!(drop, vec!["find"]);
        assert!(make.is_empty());
    }

    #[test]
    fn a_child_that_is_still_wanted_is_left_alone() {
        let want = [child("find"), child("replace")];
        let (drop, make) = reconcile(&["find"], &want.iter().collect::<Vec<_>>());
        assert!(drop.is_empty());
        assert_eq!(make, vec![1]);
    }

    #[test]
    fn one_child_swapped_for_another() {
        let want = [child("replace")];
        let (drop, make) = reconcile(&["find"], &want.iter().collect::<Vec<_>>());
        assert_eq!(drop, vec!["find"]);
        assert_eq!(make, vec![0]);
    }

    #[test]
    fn a_shown_main_window_that_is_missing_is_created() {
        assert_eq!(main_wanted(false, false), Some(true));
    }

    #[test]
    fn a_hidden_main_window_that_exists_is_dropped() {
        assert_eq!(main_wanted(true, true), Some(false));
    }

    #[test]
    fn a_main_window_that_already_matches_is_left_alone() {
        assert_eq!(main_wanted(false, true), None);
        assert_eq!(main_wanted(true, false), None);
    }

    #[test]
    fn a_ping_from_another_thread_wakes_the_loop() {
        use smithay_client_toolkit::reexports::calloop::{EventLoop, ping::make_ping};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let mut event_loop: EventLoop<()> = EventLoop::try_new().expect("event loop");
        let (ping, source) = make_ping().expect("ping");
        let woke = Arc::new(AtomicBool::new(false));
        let flag = woke.clone();
        event_loop
            .handle()
            .insert_source(source, move |_, _, _: &mut ()| {
                flag.store(true, Ordering::SeqCst);
            })
            .expect("ping source");

        let waker = Waker(Arc::new(ping));
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            waker.wake();
        });

        for _ in 0..20 {
            event_loop
                .dispatch(Duration::from_millis(50), &mut ())
                .expect("dispatch");
            if woke.load(Ordering::SeqCst) {
                return;
            }
        }
        panic!("the ping never reached the loop");
    }

    #[test]
    fn a_protocol_error_ends_the_loop_instead_of_spinning() {
        use std::io::Write;
        use std::os::unix::net::UnixStream;
        let (client, mut server) = UnixStream::pair().expect("socket pair");
        let conn = Connection::from_socket(client).expect("connection");
        let mut event_loop: EventLoop<()> = EventLoop::try_new().expect("event loop");
        WaylandSource::new(conn.clone(), conn.new_event_queue::<()>())
            .insert(event_loop.handle())
            .expect("wayland source");
        // wl_display.error(object 1, code 0, "boom"), then the compositor hangs up.
        let words: [u32; 7] = [1, 28 << 16, 1, 0, 5, u32::from_le_bytes(*b"boom"), 0];
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        server.write_all(&bytes).expect("error event");
        drop(server);
        for _ in 0..100 {
            if let Err(e) = turn(&mut event_loop, &mut (), &conn) {
                assert!(
                    e.contains("protocol error"),
                    "the loop ends on the protocol error, got {e}"
                );
                return;
            }
        }
        panic!("the loop kept turning after the protocol error");
    }
}
