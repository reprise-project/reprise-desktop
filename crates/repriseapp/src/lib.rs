//! Wayland client host for reprisekit programs: an [`App`] draws into a [`Canvas`] and takes
//! [`Event`]s; `run` gives each window an xdg toplevel and a `wl_shm` buffer.

pub mod about;
mod access;
pub mod decode;
pub mod dnd;
pub mod fielddrop;
pub mod filedlg;
pub mod heads;
pub mod keys;
pub mod oneshot;
pub mod openwith;
mod outputmgmt;
mod textinput;
pub mod unlock;

use smithay_client_toolkit::reexports::calloop::channel as reprise_channel;
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
        data_offer::{DataOfferHandler, DragOffer},
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
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
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

pub use dnd::{Accept, Actions, DragAction, DragOut, DropKind, Dropped, Offered, Uri};
pub use fielddrop::{DropForm, FileDrop};
use reprisefs::pipe::PasteBuf;
pub use reprisekit;
use reprisekit::access::{Tree, accesskit};
use reprisekit::icontheme::{IconCache, IconTheme};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::textedit::{EditKey, key_for};
use reprisekit::theme::Theme;
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
    pub text: TextRenderer,
    pub icons: IconCache,
    pub caret_on: bool,
    /// The window's client size, logical px.
    pub size: (i32, i32),
    pub maximized: bool,
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
        let base = self.theme.fonts.size_px;
        reprisekit::theme::TextUnits::new(reprisekit::theme::text_px(base, self.text_size), base)
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileClip {
    pub cut: bool,
    pub paths: Vec<PathBuf>,
}

/// `x-special/gnome-copied-files`: `cut`/`copy` on its own line, then the
/// `file://` URIs.
pub const GNOME_FILES: &str = "x-special/gnome-copied-files";
/// `text/uri-list`: the portable fallback, one `file://` URI per line
/// (`\r\n`-terminated), no cut/copy of its own.
pub const URI_LIST: &str = "text/uri-list";

/// Encode a file clip for `mime`. The `x-special` form is `cut` or `copy`
/// followed by the paths as `file://` URIs joined with `\n`, no
/// trailing newline; the URI-list form is each URI followed by `\r\n`.
pub fn encode_file_clip(c: &FileClip, mime: &str) -> Vec<u8> {
    let uris: Vec<String> = c
        .paths
        .iter()
        .map(|p| format!("file://{}", reprisefs::trash::percent_encode(p)))
        .collect();
    if mime == GNOME_FILES {
        let mut out = if c.cut {
            b"cut\n".to_vec()
        } else {
            b"copy\n".to_vec()
        };
        out.extend(uris.join("\n").into_bytes());
        out
    } else {
        let mut out = Vec::new();
        for uri in uris {
            out.extend(uri.into_bytes());
            out.extend_from_slice(b"\r\n");
        }
        out
    }
}

/// Decode a file clip from `mime`'s bytes. Lines that are blank, `#`
/// comments or not a local `file://` URI are skipped; a URI list has no
/// cut/copy line, so it decodes as a copy.
pub fn decode_file_clip(bytes: &[u8], mime: &str) -> FileClip {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.split('\n').map(|l| l.trim_end_matches('\r'));
    let cut = mime == GNOME_FILES && matches!(lines.next(), Some("cut"));
    let paths = lines
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(file_uri_path)
        .collect();
    FileClip { cut, paths }
}

/// The path a `file://` URI names on this machine: its host is empty or
/// `localhost`, and its path absolute. A URI for another host is None.
fn file_uri_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let (host, path) = rest.split_at(rest.find('/')?);
    if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) {
        return None;
    }
    Some(decode_uri(path)).filter(|p| p.is_absolute())
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn decode_uri(uri: &str) -> PathBuf {
    let bytes = uri.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
        {
            out.push(hi * 16 + lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    PathBuf::from(OsStr::from_bytes(&out))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reply {
    pub redraw: bool,
    pub redraw_all: bool,
    pub quit: bool,
    pub copy: Option<String>,
    /// Arrives as `Event::Pasted` on the window this reply is for.
    pub paste: bool,
    /// Run each argument vector as a process of its own, no shell
    /// between, detached and reaped in the background. No activation
    /// token is sent: a launched window takes focus as a child of the
    /// focused caller.
    pub spawn: Vec<Vec<OsString>>,
    pub copy_files: Option<FileClip>,
    /// Arrives as `Event::PastedFiles` on the window this reply is for.
    pub paste_files: bool,
    /// An xdg-activation token another process handed over with a request
    /// to show the main window: presented for it, so the compositor
    /// focuses it. Dropped while the main window is hidden.
    pub activate: Option<String>,
    /// Start a drag of files from the window this reply is for. It must
    /// answer a pointer press still held (the host uses that press's
    /// serial); the compositor refuses it otherwise.
    pub start_drag: Option<Box<DragOut>>,
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
    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx);
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
    /// The file dialog in window `win`, whose File name field takes a drag of text or files as
    /// a form's field does; files go in by name, or by path from another folder. None (the
    /// default) hands the drags to `event`.
    fn drop_file_dialog(&mut self, _win: WinId) -> Option<FileDrop<'_>> {
        None
    }
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
    use reprisekit::frame::{Chrome, FrameGeometry};
    CAPTION_FIT.with_borrow(|(theme, output_w)| {
        let Some(theme) = theme else {
            return 0;
        };
        let fit = reprisekit::look::caption_fit(theme, title, Chrome::DIALOG);
        match output_w {
            Some(w) => {
                let usable = FrameGeometry::client_size_for_total(
                    &theme.metrics,
                    *w,
                    1,
                    false,
                    Chrome::DIALOG,
                );
                fit.min(usable.0)
            }
            None => fit,
        }
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
/// size, a resizable one at least `min`.
fn regrown(now: (i32, i32), min: (i32, i32), max: Option<(i32, i32)>) -> (i32, i32) {
    max.unwrap_or((now.0.max(min.0), now.1.max(min.1)))
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
/// [`reprisekit::blue::ENV`], Glass mode from
/// [`reprisekit::glass::MODE_ENV`]), else [`Theme::default`]; then a
/// [`reprisekit::theme::COLORS_ENV`] scheme is laid over it.
fn theme_from_env(
    interface: Option<&str>,
    variant: Option<&str>,
    glass_mode: Option<&str>,
    colors: Option<&str>,
) -> Theme {
    let mut theme = reprisekit::interfaces::theme_with(
        reprisekit::interface::Interface::from_env_value(interface),
        reprisekit::blue::Variant::from_env_value(variant),
        &reprisekit::glass::Settings {
            mode: glass_mode
                .and_then(reprisekit::glass::Mode::parse)
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
    colors: Option<String>,
    icon_theme: String,
    /// `[fonts] shell_dlg`, read from `config.toml` at the start and
    /// followed through the portal after: the second dialog font every
    /// renderer loads beside the UI font.
    shell_dlg: Option<String>,
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
            colors: std::env::var(reprisekit::theme::COLORS_ENV).ok(),
            icon_theme: std::env::var(reprisekit::icontheme::ENV)
                .unwrap_or_else(|_| "hicolor".into()),
            shell_dlg: repriseconf::shell_dlg(),
            text_size: repriseconf::text_size(),
            double_click_ms: repriseconf::double_click_ms(),
        }
    }

    fn theme(&self) -> Theme {
        let mut theme = theme_from_env(
            self.interface.as_deref(),
            self.blue_variant.as_deref(),
            self.glass_mode.as_deref(),
            self.colors.as_deref(),
        );
        theme.fonts.shell_dlg = self.shell_dlg.clone();
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

pub fn run(app: Box<dyn App>) {
    let conn = Connection::connect_to_env().expect("WAYLAND_DISPLAY");
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
    // The main window itself is `sync_main`'s, so a program that starts
    // hidden never gets a surface for it.
    let ((w, h), _) = opening_size(&*app);
    let look = Look::from_env();
    let theme = look.theme();
    publish_theme(&theme);
    let text = TextRenderer::load(&theme.fonts);
    let theme = sized(theme, &*app, look.text_size);
    let icons = look.icons();
    let (ping, ping_source) = make_ping().expect("ping");
    let (a11y, a11y_source) = channel();
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
        copy_text: Arc::default(),
        copy_clip: None,
        drag_out: None,
        drag_in: None,
        field_drag: None,
        asked_drop: None,
        ticking: Vec::new(),
        portal_done,
        press_serial: 0,
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
        },
        exit: false,
        last_serial: 0,
        a11y,
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
        if let Err(e) = event_loop.dispatch(None, &mut host) {
            log::error!("wayland connection lost: {e}; exiting");
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
        if host.exit {
            break;
        }
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
}

impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(v) = self.scaling.viewport.take() {
            v.destroy();
        }
        if let Some(f) = self.scaling.fraction.take() {
            f.destroy();
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
    copy_text: Arc<[u8]>,
    copy_clip: Option<FileClip>,
    drag_out: Option<Outgoing>,
    drag_in: Option<Incoming>,
    /// A drag over a window's `App::drop_form`.
    field_drag: Option<(WinId, fielddrop::Hover)>,
    asked_drop: Option<Asked>,
    ticking: Vec<WinId>,
    /// Where a document portal call's thread sends what to do with its
    /// answer (`reprisebus::filetransfer`).
    portal_done: Sender<OnLoop>,
    /// The last pointer press's serial, which a drag must name.
    press_serial: u32,
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

    fn modal(&self) -> Option<WinId> {
        top_modal(self.slots.iter().map(|s| (s.id, s.modal)))
    }

    fn gated(&self, win: WinId, ev: &Event) -> bool {
        let flyout = self
            .slots
            .iter()
            .any(|s| s.id == win && s.window.toplevel().is_none());
        input_blocked(ev, self.modal(), win, flyout)
    }

    fn send(&mut self, win: WinId, ev: Event) {
        if self.gated(win, &ev) {
            return;
        }
        match self.slots.iter().find(|s| s.id == win) {
            Some(s) => {
                self.ctx.size = s.size;
                self.ctx.maximized = s.maximized;
                self.ctx.origin = s.origin;
            }
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
        if let Some((reply, changed)) = self.field_drop(win, &ev) {
            self.apply(win, reply);
            if changed != reprisekit::controls::form::Event::None {
                self.send(win, Event::FieldDropped(changed));
            }
            return;
        }
        let mut reply = self.app.event(win, ev, &mut self.ctx);
        reply.tick |= self.ctx.want_tick.take();
        self.apply(win, reply);
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
        let theme_px = self.ctx.theme.fonts.size_px;
        let out = if let Some(DropForm { form, px }) = self.app.drop_form(win) {
            let px = px.unwrap_or_else(|| form.units().px(theme_px));
            fielddrop::event(form, &mut hover, ev, &self.ctx.text, px)
        } else {
            let mut file = self.app.drop_file_dialog(win)?;
            fielddrop::event(&mut file, &mut hover, ev, &self.ctx.text, theme_px)
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
        if reply.quit {
            self.exit = true;
        }
        // Diffed after every event, not just `win`'s: a dialog's event
        // (Open, Save As) often retitles the main window.
        let title = self.app.title();
        let caption = self.app.caption(MAIN);
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
                send_tag(tags.as_ref(), w, caption);
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
        if reply.paste_files {
            self.paste_files(win);
        }
        for argv in reply.spawn {
            self.spawn(&argv);
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
        if reply.resize {
            self.begin_resize(win);
        }
        let fixed = self.app.fixed();
        let mut resized = false;
        if let Some(size) = reply.size
            && let Some(s) = self.slot_mut(MAIN)
        {
            let size = bounded(sane_size(size), s.bounds);
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
        if let (Some(token), Some(activation)) = (reply.activate, &self.activation)
            && let Some(main) = self.slots.iter().find(|s| s.id == MAIN)
        {
            activation.activate::<Host>(main.window.wl_surface(), token);
        }
        self.drain_head_requests();
    }

    fn send_tag(&self, window: &Window, caption: Caption) {
        if caption != Caption::Window {
            send_tag(self.tag_manager.as_ref(), window, caption);
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
        self.send_tag(&window, caption);
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
                activated: true,
                configured: false,
                dirty: true,
                frame_pending: false,
                mapped: false,
                modal: false,
                caption,
                access: access::SlotAccess::new(MAIN, &self.a11y),
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

    /// Fit window `id` to what its program asks now, after that changed (the text size, a
    /// child's size): a fixed window takes its size, a resizable one grows to at least its
    /// minimum. The compositor keeps its top-left; a maximized window keeps the screen's size.
    /// A window that changed size hears `Event::Resized`.
    fn refit(&mut self, id: WinId) {
        let Some((min, max)) = self.limits(id) else {
            return;
        };
        self.declare_limits(id);
        let Some(s) = self.slot_mut(id) else {
            return;
        };
        let size = regrown(s.size, min, max);
        if s.maximized || size == s.size {
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
        self.slots.retain(|s| s.id == MAIN || !drop.contains(&s.id));
        let main = self.slots.iter().find(|s| s.id == MAIN);
        // xdg-shell drops a parent that is not mapped yet, so a child
        // wanted before the main window's first frame waits for it
        // (`draw` calls back here then).
        if main.is_some_and(|s| !s.mapped) {
            return;
        }
        let parent = main
            .and_then(|s| s.window.toplevel())
            .map(|w| w.xdg_toplevel().clone());
        let app_id = self.app.app_id();
        for i in make {
            let c = &want[i];
            // `reconcile` does not dedupe `want`; one surface per id.
            if self.slots.iter().any(|s| s.id == c.id) {
                continue;
            }
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
            let caption = self.app.caption(c.id);
            self.send_tag(&window, caption);
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
                activated: true,
                configured: false,
                dirty: true,
                frame_pending: false,
                mapped: false,
                modal: c.modal,
                caption,
                access: access::SlotAccess::new(c.id, &self.a11y),
            });
        }
        let mut resized = Vec::new();
        for c in &want {
            let caption = self.app.caption(c.id);
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
                    send_tag(self.tag_manager.as_ref(), w, caption);
                }
            }
        }
        for id in resized {
            self.refit(id);
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
            title: String::new(),
            size: (0, 0),
            origin,
            maximized: false,
            asked: (None, None),
            own: false,
            bounds: None,
            wanted: (0, 0),
            activated: true,
            configured: false,
            dirty: true,
            frame_pending: false,
            mapped: false,
            modal: false,
            caption: Caption::Window,
            access: access::SlotAccess::new(f.id, &self.a11y),
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
        self.ctx.size = s.size;
        self.ctx.maximized = s.maximized;
        self.ctx.origin = s.origin;
        let (app, ctx) = (&mut self.app, &self.ctx);
        let acc = &mut self.slots[i].access;
        acc.adapter.update_if_active(|| {
            let mut tree = Tree::window(&title, size);
            if modal && let Some(root) = tree.get_mut(reprisekit::access::ROOT) {
                root.set_role(accesskit::Role::Dialog);
                root.set_modal();
            }
            app.access(id, &mut tree, ctx);
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
        // The old theme's fonts at 100%, as the renderer was loaded with them.
        let mut before = self.ctx.theme.fonts.clone();
        before.size_px = self.ctx.theme.text.base_px(before.size_px);
        if theme.fonts != before {
            self.ctx.text = TextRenderer::load(&theme.fonts);
        }
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
                let target = (self.slot_mut(win))
                    .filter(|s| s.access.state.active)
                    .and_then(|s| s.access.state.published.target(req.node));
                let Some(g) = target.and_then(|t| reprisekit::access::gesture(req.act, t)) else {
                    return;
                };
                let apart = self.ctx.theme.metrics.double_click_ms.max(0) as u32 + 1000;
                self.press_time = self.press_time.wrapping_add(apart);
                for input in access::reader_input(g, self.press_time) {
                    self.pointer(win, input);
                }
            }
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
        let win = self.focus.unwrap_or(MAIN);
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

    fn copy_files(&mut self, clip: FileClip) {
        let (Some(manager), Some(device)) = (&self.data_device_manager, &self.data_device) else {
            return;
        };
        let source = manager.create_copy_paste_source(&self.qh, [GNOME_FILES, URI_LIST]);
        source.set_selection(device, self.last_serial);
        self.copy_clip = Some(clip);
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
        let mime = [TEXT_MIME, "text/plain", "UTF8_STRING"]
            .iter()
            .find(|m| offer.with_mime_types(|types| types.iter().any(|t| t == *m)))
            .map(|m| m.to_string());
        let Some(mime) = mime else {
            return;
        };
        let Ok(read_pipe) = offer.receive(mime) else {
            return;
        };
        self.read_offer(win, read_pipe, |buf| Event::Pasted(buf.into_text()));
    }

    fn paste_files(&mut self, win: WinId) {
        let Some(device) = &self.data_device else {
            return;
        };
        let Some(offer) = device.data().selection_offer() else {
            return;
        };
        let mime = [GNOME_FILES, URI_LIST]
            .iter()
            .find(|m| offer.with_mime_types(|types| types.iter().any(|t| t == *m)))
            .map(|m| m.to_string());
        let Some(mime) = mime else {
            return;
        };
        let Ok(read_pipe) = offer.receive(mime.clone()) else {
            return;
        };
        self.read_offer(win, read_pipe, move |buf| {
            Event::PastedFiles(decode_file_clip(buf.bytes(), &mime))
        });
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
        let Some((program, args)) = argv.split_first() else {
            return;
        };
        match Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(e) => log::warn!("spawn {argv:?}: {e}"),
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
        if (canvas.width, canvas.height, canvas.scale()) != (w as u32, h as u32, scale) {
            canvas = Canvas::with_scale(w as u32, h as u32, scale);
        } else {
            canvas.pixels.fill(0);
        }
        self.ctx.size = (w, h);
        self.ctx.maximized = self.slots[i].maximized;
        self.ctx.origin = self.slots[i].origin;
        // Only the active window draws as active (a dialog's default
        // button, under a look that shows it): a toplevel as the compositor
        // says, a flyout while it has the keyboard.
        self.ctx.theme.active = match self.slots[i].window {
            Shell::Toplevel(_) => self.slots[i].activated,
            Shell::Layer(_) => self.focus == Some(id),
        };
        let preedit = self
            .text_input
            .as_ref()
            .filter(|t| t.entered == Some(id))
            .and_then(|t| t.preedit().cloned());
        canvas.set_preedit(preedit);
        let app = &mut self.app;
        self.ctx
            .at_scale(scale, |ctx| app.draw(id, &mut canvas, ctx));
        self.ctx.theme.active = true;
        let spot = canvas.take_spot();
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
        surface.damage_buffer(0, 0, bw, bh);
        surface.frame(qh, FrameCallbackData(surface.clone()));
        s.frame_pending = true;
        s.buffers[k]
            .as_ref()
            .expect("buffer")
            .attach_to(surface)
            .expect("attach");
        s.window.commit();
        if !std::mem::replace(&mut s.mapped, true) && id == MAIN {
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
        self.rescale(surface, None, Some(factor));
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
        let Some(id) = self.slot_of(surface) else {
            return;
        };
        self.focus = Some(id);
        if let Some(s) = self.slot_mut(id) {
            s.access.adapter.update_window_focus_state(true);
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
        let Some(id) = self.slot_of(surface) else {
            return;
        };
        if self.focus == Some(id) {
            self.focus = None;
        }
        if let Some(s) = self.slot_mut(id) {
            s.access.adapter.update_window_focus_state(false);
        }
        self.redraw_if_active_shows(id);
        self.send(id, Event::Focus(false));
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
            let Some(id) = self.slot_of(&event.surface) else {
                continue;
            };
            let (x, y) = (event.position.0 as i32, event.position.1 as i32);
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
                    self.press_time = time;
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
    _icon: Option<DragIcon>,
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

impl DragIcon {
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
            _icon: icon,
            source,
        });
    }

    fn drag_icon(&self, image: &Canvas, grab: (i32, i32)) -> Option<DragIcon> {
        let (dw, dh) = image.device_size();
        if dw == 0 || dh == 0 || dw > MAX_SIDE as u32 || dh > MAX_SIDE as u32 {
            return None;
        }
        let (dw, dh) = (dw as i32, dh as i32);
        let mut pool = SlotPool::new((dw * dh * 4) as usize, &self.shm).ok()?;
        let (buffer, pixels) = pool
            .create_buffer(dw, dh, dw * 4, wl_shm::Format::Argb8888)
            .ok()?;
        copy_pixels(pixels, &image.pixels);
        Some(DragIcon {
            surface: self.compositor.create_surface(&self.qh),
            buffer,
            _pool: pool,
            scale: image.scale(),
            grab,
        })
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
                    host.peeked(&offer, decode_file_clip(buf.bytes(), URI_LIST).paths);
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
            if mime == GNOME_FILES || mime == URI_LIST {
                match &self.copy_clip {
                    Some(clip) => encode_file_clip(clip, &mime).into(),
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
    type RequestUdata = ();
    fn new_token(&mut self, _token: String, _data: &RequestData<()>) {}
}

struct NoEvents;

fn send_tag(
    manager: Option<&xdg_toplevel_tag_manager_v1::XdgToplevelTagManagerV1>,
    window: &Window,
    caption: Caption,
) {
    if let Some(m) = manager {
        let tag = match caption {
            Caption::Window => "",
            Caption::Prompt => "prompt",
            Caption::Help => "help-dialog",
            Caption::Fixed => "fixed-dialog",
        };
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
    zxdg_importer_v2::ZxdgImporterV2,
    zxdg_importer_v1::ZxdgImporterV1,
    xdg_wm_dialog_v1::XdgWmDialogV1,
    xdg_dialog_v1::XdgDialogV1,
    xdg_toplevel_tag_manager_v1::XdgToplevelTagManagerV1,
    zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1,
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

    /// Redraw flyout `id`, which just gained or lost the keyboard, when
    /// the look draws an inactive window's default button apart. A
    /// toplevel redraws on the configure that changes its `activated`.
    fn redraw_if_active_shows(&mut self, id: WinId) {
        let flyout = self
            .slots
            .iter()
            .any(|s| s.id == id && matches!(s.window, Shell::Layer(_)));
        if flyout
            && self
                .ctx
                .theme
                .look
                .style()
                .is_some_and(|st| st.default_needs_active)
        {
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
            theme_from_env(None, None, None, None),
            Theme::default(),
            "no REPRISE_INTERFACE and no REPRISE_COLORS must give Theme::default()"
        );
    }

    #[test]
    fn theme_from_env_with_blue_takes_the_blue_style() {
        let t = theme_from_env(Some("blue"), None, None, None);
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
            let t = theme_from_env(Some("blue"), Some(v.as_str()), None, None);
            assert_eq!(t.look, blue_theme(v).look, "{v:?}");
            assert_eq!(t.colors, blue_theme(v).colors, "{v:?}");
        }
        for unknown in [None, Some("green"), Some("Silver")] {
            assert_eq!(
                theme_from_env(Some("blue"), unknown, None, None).look,
                blue_theme(Variant::Blue).look,
                "{unknown:?} is navy"
            );
        }
        assert_eq!(
            theme_from_env(Some("classic"), Some("olive"), None, None),
            Theme::default(),
            "the variant means nothing outside Blue"
        );
    }

    #[test]
    fn the_glass_mode_picks_dark_glass_and_anything_else_is_light() {
        let dark = theme_from_env(Some("glass"), None, Some("dark"), None);
        let light = theme_from_env(Some("glass"), None, None, None);
        assert_ne!(dark.look, light.look, "dark Glass is another look");
        assert_eq!(dark.look.style().unwrap().name, "glass-dark");
        for odd in [Some("Dark"), Some(""), Some("dim")] {
            assert_eq!(
                theme_from_env(Some("glass"), None, odd, None).look,
                light.look,
                "{odd:?} is light"
            );
        }
        assert_eq!(
            theme_from_env(Some("classic"), None, Some("dark"), None).look,
            theme_from_env(Some("classic"), None, None, None).look,
            "Classic ignores the glass mode"
        );
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
        let t = theme_from_env(Some("blue"), None, None, Some("window_text=#010203"));
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
            text: TextRenderer::load(&theme.fonts),
            icons: IconCache::new(IconTheme::load("test", &[])),
            caret_on: true,
            size: (40, 30),
            maximized: false,
            origin: (0, 0),
            wake: Waker::noop(),
            heads: Default::default(),
            want_tick: Default::default(),
            text_size: 150,
        };
        let grown = reprisekit::theme::TextUnits::new(17, 11);
        assert_eq!(ctx.forms().fonts.size_px, 17);
        assert_eq!(ctx.forms().text, grown);
        ctx.theme = theme.with_text_size(150);
        assert_eq!(ctx.forms().fonts.size_px, 17);
        assert_eq!(ctx.forms().text, grown);
    }

    #[test]
    fn a_window_at_scale_2_paints_glass_parts_at_2() {
        use reprisekit::interface::Interface;
        use reprisekit::look::widgets::draw_close_box;
        use reprisekit::raster::{Canvas, Rect};
        let theme = reprisekit::interfaces::theme_for(Interface::Glass);
        let mut ctx = Ctx {
            theme: theme.clone(),
            text: TextRenderer::load(&theme.fonts),
            icons: IconCache::new(IconTheme::load("test", &[])),
            caret_on: true,
            size: (40, 30),
            maximized: false,
            origin: (0, 0),
            wake: Waker::noop(),
            heads: Default::default(),
            want_tick: Default::default(),
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
            text: TextRenderer::load(&theme.fonts),
            icons: IconCache::new(IconTheme::load("test", &[])),
            caret_on: true,
            size: (120, 30),
            maximized: false,
            origin: (0, 0),
            wake: Waker::noop(),
            heads: Default::default(),
            want_tick: Default::default(),
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
            decode_file_clip(list, URI_LIST).paths,
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
    fn a_flyout_is_made_dropped_or_left() {
        let f = Flyout {
            id: "flyout",
            at: (10, 20),
        };
        assert_eq!(flyout_wanted(Some(f), None), (None, Some(f)));
        assert_eq!(flyout_wanted(None, Some("flyout")), (Some("flyout"), None));
        assert_eq!(flyout_wanted(Some(f), Some("flyout")), (None, None));
        let g = Flyout {
            id: "other",
            at: (0, 0),
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

    /// A text size change grows an open window: a fixed one to the size its program asks
    /// for, a resizable one to at least its new minimum, never smaller than it was.
    #[test]
    fn an_open_window_grows_to_what_its_program_asks() {
        assert_eq!(
            regrown((398, 424), (398, 424), Some((597, 636))),
            (597, 636)
        );
        assert_eq!(regrown((900, 700), (816, 792), None), (900, 792));
        assert_eq!(regrown((900, 900), (816, 792), None), (900, 900));
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
    fn file_clip_round_trips_both_mime_types() {
        let c = FileClip {
            cut: true,
            paths: vec![PathBuf::from("/a b/c.txt")],
        };
        assert_eq!(
            encode_file_clip(&c, GNOME_FILES),
            b"cut\nfile:///a%20b/c.txt"
        );
        assert_eq!(encode_file_clip(&c, URI_LIST), b"file:///a%20b/c.txt\r\n");
        assert_eq!(
            decode_file_clip(b"copy\nfile:///x\nfile:///y", GNOME_FILES)
                .paths
                .len(),
            2
        );
        assert!(!decode_file_clip(b"file:///x\r\n", URI_LIST).cut);
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
}
