//! Wayland client host for reprisekit programs: an [`App`] draws into a [`Canvas`] and takes
//! [`Event`]s; `run` gives each window an xdg toplevel and a `wl_shm` buffer.

pub mod about;
mod access;
mod app;
mod clipboard;
mod ctx;
pub mod decode;
mod deliver;
pub mod dnd;
mod drag;
mod draw;
mod editmenu;
mod event;
pub mod fielddrop;
pub mod filedlg;
mod hangup;
pub mod heads;
mod input;
pub mod keys;
pub mod listicons;
pub mod listjob;
mod look;
mod menupopup;
pub mod oneshot;
pub mod openwith;
mod outputmgmt;
mod primary;
mod reply;
mod runloop;
mod sizing;
mod tabgroup;
mod textinput;
pub mod thumbs;
pub mod unlock;
mod windows;

use drag::{Asked, Incoming, Outgoing};
use look::Look;
use reprisekit::cues::Cues;
use reprisekit::raster::Canvas;
use reprisekit::textedit::TextSpot;
use smithay_client_toolkit::reexports::protocols::ext::background_effect::v1::client::{
    ext_background_effect_manager_v1, ext_background_effect_surface_v1,
};
use smithay_client_toolkit::reexports::protocols::wp::{
    fractional_scale::v1::client::{wp_fractional_scale_manager_v1, wp_fractional_scale_v1},
    keyboard_shortcuts_inhibit::zv1::client::{
        zwp_keyboard_shortcuts_inhibit_manager_v1, zwp_keyboard_shortcuts_inhibitor_v1,
    },
    text_input::zv3::client::zwp_text_input_manager_v3,
    viewporter::client::{wp_viewport, wp_viewporter},
};
use smithay_client_toolkit::reexports::protocols::xdg::{
    dialog::v1::client::xdg_wm_dialog_v1,
    foreign::{zv1::client::zxdg_importer_v1, zv2::client::zxdg_importer_v2},
    toplevel_tag::v1::client::xdg_toplevel_tag_manager_v1,
};
use smithay_client_toolkit::{
    activation::ActivationState,
    compositor::CompositorState,
    data_device_manager::{
        DataDeviceManagerState, data_device::DataDevice, data_source::CopyPasteSource,
    },
    output::OutputState,
    reexports::calloop::{LoopHandle, channel::Sender},
    registry::RegistryState,
    seat::{SeatState, keyboard::Modifiers},
    shell::{
        WaylandSurface,
        wlr_layer::{LayerShell, LayerSurface},
        xdg::{XdgShell, window::Window},
    },
    shm::{
        Shm,
        slot::{Buffer, SlotPool},
    },
};
use std::sync::Arc;
use tabgroup::reprise_tabs_v1;
use wayland_client::{
    QueueHandle,
    protocol::{wl_keyboard, wl_pointer, wl_seat, wl_surface},
};
use windows::Foreign;

pub use access::{Reader, ReaderForm};
pub use app::{
    App, Caption, Child, Flyout, MAIN, Start, WinId, reader_forms, reader_popup, window_id,
};
pub use clipboard::{
    clipboard_has_files, clipboard_has_text, fake_clipboard_files, fake_clipboard_text,
};
pub use ctx::{Ctx, Poster, Waker};
pub use deliver::TICK_MS;
pub use dnd::{Accept, Actions, DragAction, DragOut, DropKind, Dropped, Offered, Uri};
pub use event::{BTN_LEFT, BTN_RIGHT, Event, KeyInput, Mods, PointerInput};
pub use fielddrop::{DropForm, FileDrop};
pub use look::{caption_fit, dialog_max_w, start_forms, start_text, start_theme};
pub use reply::{Reply, spawn_in};
pub use reprisefs::dropdata::{GNOME_FILES, URI_LIST};
pub use reprisefs::fileclip::FileClip;
pub use reprisekit;
pub use runloop::{applet_main, run};
pub use sizing::MIN_SIZE;
pub use windows::tab_groups;

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
    /// The seat has a keyboard `bind_keyboard` has not bound yet.
    keyboard_due: bool,
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
    /// Ends the thread watching the icon theme's directories when replaced.
    icon_watch: Option<reprisefs::dirchange::Stop>,
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

    fn slot_of_id(&self, win: WinId) -> bool {
        self.slots.iter().any(|s| s.id == win)
    }

    fn mark_dirty(&mut self, id: WinId) {
        if let Some(s) = self.slot_mut(id) {
            s.dirty = true;
        }
    }
}

type OnLoop = Box<dyn FnOnce(&mut Host) + Send>;

#[cfg(test)]
mod tests;
