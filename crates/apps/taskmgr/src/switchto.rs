//! Switch To: raises a window of a program through wlr-foreign-toplevel-management, on a
//! short-lived Wayland connection of its own (the compositor's taskbar path for docks).

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop, event_created_child};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};

#[derive(Default)]
struct State {
    handles: Vec<(ZwlrForeignToplevelHandleV1, String)>,
}

/// Activates the first window whose app id `wanted` accepts. Blocks for two round trips.
pub fn activate(wanted: impl Fn(&str) -> bool) -> Result<bool, String> {
    let conn = Connection::connect_to_env().map_err(|e| e.to_string())?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(|e| e.to_string())?;
    let qh = queue.handle();
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).map_err(|e| e.to_string())?;
    let manager: ZwlrForeignToplevelManagerV1 = globals
        .bind(&qh, 1..=3, ())
        .map_err(|e| format!("zwlr_foreign_toplevel_manager_v1: {e}"))?;
    let mut state = State::default();
    for _ in 0..2 {
        queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
    }
    let hit = state.handles.iter().position(|(_, id)| wanted(id));
    if let Some(i) = hit {
        state.handles[i].0.activate(&seat);
    }
    manager.stop();
    queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
    Ok(hit.is_some())
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(State: ignore wl_seat::WlSeat);

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } = event {
            state.handles.push((toplevel, String::new()));
        }
    }

    event_created_child!(State, ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ())
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        h: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                if let Some((_, id)) = state.handles.iter_mut().find(|(x, _)| x == h) {
                    *id = app_id;
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                state.handles.retain(|(x, _)| x != h);
                h.destroy();
            }
            _ => {}
        }
    }
}
