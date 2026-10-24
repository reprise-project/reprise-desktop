//! Connecting to the compositor and running the event loop.

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
        if host.exit {
            let _ = conn.flush();
            hangup::hang_up(conn.backend().poll_fd());
            break;
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
