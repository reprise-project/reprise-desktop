//! Connecting to the compositor and running the event loop.

use crate::Slot;
use crate::deliver::blink_target;
use crate::look::{Look, publish_theme, sized, text_for};
use crate::sizing::opening_size;
use crate::tabgroup::reprise_tabs_v1;
use crate::windows::TAB_GROUPS;
use crate::{
    App, Ctx, Event, Host, MAIN, OnLoop, Waker, WinId, access, filedlg, hangup, heads, outputmgmt,
    primary,
};
use smithay_client_toolkit::reexports::calloop::channel as reprise_channel;
use smithay_client_toolkit::reexports::protocols::ext::background_effect::v1::client::{
    ext_background_effect_manager_v1, ext_background_effect_surface_v1,
};
use smithay_client_toolkit::reexports::protocols::wp::{
    fractional_scale::v1::client::wp_fractional_scale_manager_v1,
    keyboard_shortcuts_inhibit::zv1::client::zwp_keyboard_shortcuts_inhibit_manager_v1,
    text_input::zv3::client::zwp_text_input_manager_v3,
    viewporter::client::{wp_viewport, wp_viewporter},
};
use smithay_client_toolkit::reexports::protocols::xdg::{
    dialog::v1::client::{xdg_dialog_v1, xdg_wm_dialog_v1},
    foreign::{zv1::client::zxdg_importer_v1, zv2::client::zxdg_importer_v2},
    toplevel_tag::v1::client::xdg_toplevel_tag_manager_v1,
};
use smithay_client_toolkit::{
    activation::ActivationState,
    compositor::CompositorState,
    data_device_manager::DataDeviceManagerState,
    delegate_dispatch2, delegate_registry,
    dispatch2::Dispatch2,
    output::OutputState,
    reexports::{
        calloop::{
            EventLoop,
            channel::channel,
            ping::make_ping,
            timer::{TimeoutAction, Timer},
        },
        calloop_wayland_source::WaylandSource,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{SeatState, keyboard::Modifiers},
    shell::{wlr_layer::LayerShell, xdg::XdgShell},
    shm::Shm,
};
use std::sync::Arc;
use std::time::Duration;
use wayland_client::{Connection, QueueHandle, globals::registry_queue_init};

const BLINK_MS: u64 = 530;

impl Host {
    /// Puts the caret blink timer in the loop when a window takes blinks and it is not there.
    pub(crate) fn arm_blink(&mut self) {
        if self.blink_armed || !self.slots.iter().any(Slot::blinks) {
            return;
        }
        let timer = Timer::from_duration(Duration::from_millis(BLINK_MS));
        match (self.loop_handle).insert_source(timer, |_, _, host: &mut Host| host.blink()) {
            Ok(_) => {
                self.blink_armed = true;
                self.ctx.caret_on = true;
            }
            Err(e) => log::warn!("caret blink timer: {e}"),
        }
    }

    /// One blink: the caret flips and the keyboard's window (else the pointer's) hears it. A
    /// window that was clean redraws at once as a blink-only frame, whose damage is the caret.
    fn blink(&mut self) -> TimeoutAction {
        if !self.slots.iter().any(Slot::blinks) {
            self.blink_armed = false;
            return TimeoutAction::Drop;
        }
        self.ctx.caret_on = !self.ctx.caret_on;
        let target = blink_target(self.focus, self.pointer_in)
            .and_then(|win| self.slots.iter().find(|s| s.id == win))
            .filter(|s| s.blinks())
            .map(|s| (s.id, !s.dirty && !s.frame_pending && s.configured));
        if let Some((win, clean)) = target {
            let quiet = clean && self.menu_popups.is_none();
            self.send(win, Event::Blink);
            let qh = self.qh.clone();
            if quiet
                && self
                    .slot_mut(win)
                    .is_some_and(|s| std::mem::take(&mut s.dirty))
            {
                self.draw_frame(win, &qh, true);
            }
        }
        TimeoutAction::ToDuration(Duration::from_millis(BLINK_MS))
    }
}

/// A tray applet's `main`: the journal logger, then the program `app` makes from whether `args`
/// hold `--tray` (start with the window hidden).
pub fn applet_main<A: App + 'static>(args: &[std::ffi::OsString], app: impl FnOnce(bool) -> A) {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    let tray_only = args.iter().any(|a| a == "--tray");
    run(Box::new(app(tray_only))).unwrap_or_else(|e| e.exit());
}

/// Why [`run`] could not start: no Wayland compositor to connect to, or one without a global
/// every window needs.
#[derive(Debug)]
pub struct RunError(String);

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RunError {}

fn failed<E: std::fmt::Display>(what: &'static str) -> impl FnOnce(E) -> RunError {
    move |e| RunError(format!("{what}: {e}"))
}

impl RunError {
    /// Prints the error and ends the process with status 1, for a program's `main`.
    pub fn exit(self) -> ! {
        eprintln!("{self}");
        std::process::exit(1)
    }
}

/// Runs `app` until it quits. Fails only before the first window, when the compositor cannot be
/// reached or lacks `wl_compositor`, `xdg_wm_base` or `wl_shm`.
pub fn run(app: Box<dyn App>) -> Result<(), RunError> {
    filedlg::remember_views();
    let conn =
        Connection::connect_to_env().map_err(failed("cannot connect to a Wayland compositor"))?;
    hangup::on_signals(conn.backend().poll_fd());
    let (globals, event_queue) = registry_queue_init(&conn).map_err(failed("wayland registry"))?;
    let qh = event_queue.handle();
    let mut event_queue = event_queue;
    let mut event_loop: EventLoop<Host> = EventLoop::try_new().map_err(failed("event loop"))?;
    let loop_handle = event_loop.handle();
    hangup::route_to(&loop_handle, |host: &mut Host| host.app.emergency());

    let compositor = CompositorState::bind(&globals, &qh).map_err(failed("wl_compositor"))?;
    let xdg_shell = XdgShell::bind(&globals, &qh).map_err(failed("xdg_wm_base"))?;
    let layer_shell = LayerShell::bind(&globals, &qh).ok();
    let activation = ActivationState::bind(&globals, &qh).ok();
    let shm = Shm::bind(&globals, &qh).map_err(failed("wl_shm"))?;
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
        keyboard_due: false,
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
            output_size: None,
            at_pick: false,
        },
        exit: false,
        last_serial: 0,
        a11y,
        a11y_bus: false,
        key_echo: None,
        look,
        icon_watch: None,
        blink_armed: false,
        shm_trouble: Default::default(),
        shm_retry: false,
    };
    // Started before the roundtrip, so the theme warms while the compositor answers.
    host.watch_icons();
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
    if let crate::Start::Exit(code) = host.app.start(waker) {
        shut_down(&conn);
        std::process::exit(code);
    }
    if no_outputs {
        host.send(MAIN, Event::Heads(Vec::new()));
    }
    host.sync_main();
    // A program may start with its flyout up; no event would reconcile it otherwise.
    host.sync_flyout();

    loop {
        let step = rescued(
            &mut host,
            |host| {
                // Everything that dirties a slot is a source here, so nothing needs polling.
                turn(&mut event_loop, host, &conn)?;
                let ready: Vec<WinId> = host
                    .slots
                    .iter()
                    .filter(|s| s.dirty && s.configured)
                    .map(|s| s.id)
                    .collect();
                // Before the draws: an open menu on popups is left out of its window and laid out
                // where it shows.
                host.place_menu();
                for id in ready {
                    if let Some(s) = host.slot_mut(id) {
                        s.dirty = false;
                    }
                    host.draw(id, &qh);
                }
                host.bind_keyboard(&conn, &qh);
                // After the draws: a window's draw opens the menu a right press asked for.
                host.settle_keyboard();
                host.sync_menu();
                Ok::<bool, String>(host.exit)
            },
            |host| host.app.emergency(),
        );
        match step {
            Ok(false) => {}
            Ok(true) => {
                shut_down(&conn);
                return Ok(());
            }
            Err(e) => {
                log::error!("{e}; exiting");
                host.app.emergency();
                std::process::exit(1);
            }
        }
    }
}

fn shut_down(conn: &Connection) {
    let _ = conn.flush();
    hangup::hang_up(conn.backend().poll_fd());
}

/// Runs `step` on `data`. When it panics, `rescue` gets `data` once the unwinding has released
/// every borrow of it, and the panic then carries on.
pub(crate) fn rescued<D, T>(
    data: &mut D,
    step: impl FnOnce(&mut D) -> T,
    rescue: impl FnOnce(&mut D),
) -> T {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| step(data))) {
        Ok(out) => out,
        Err(panic) => {
            rescue(data);
            std::panic::resume_unwind(panic)
        }
    }
}

/// Runs one turn of a loop holding a `WaylandSource`. The source logs and drops a
/// protocol error, and the closed socket then stays readable, so the stored error ends the loop here.
pub(crate) fn turn<D>(
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

pub(crate) struct NoEvents;

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
    wp_viewporter::WpViewporter,
    wp_viewport::WpViewport,
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    zwp_text_input_manager_v3::ZwpTextInputManagerV3,
);

delegate_dispatch2!(Host);
delegate_registry!(Host);

impl ProvidesRegistryState for Host {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState,];
}
