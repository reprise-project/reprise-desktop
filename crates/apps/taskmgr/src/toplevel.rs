//! Switch To, Minimize and Maximize on one window through wlr-foreign-toplevel-management, on a
//! short-lived Wayland connection of its own (the compositor's taskbar path for docks).

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop, event_created_child};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};

/// A window as the protocol names it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Toplevel {
    pub app_id: String,
    pub title: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Do {
    Activate,
    Minimize,
    Maximize,
}

/// The window among `ws` that the compositor listed as `w`: the same app id and title.
pub fn the_one(ws: &[Toplevel], w: &reprisebus::tray::AppWindow) -> Option<usize> {
    (ws.iter()).position(|t| t.app_id == w.app_id && t.title == w.title)
}

#[derive(Default)]
struct State {
    handles: Vec<(ZwlrForeignToplevelHandleV1, Toplevel)>,
}

/// Does `what` to the window `pick` chooses among the open ones, if it picks one. Blocks for
/// three round trips.
pub fn on_window(pick: impl Fn(&[Toplevel]) -> Option<usize>, what: Do) -> Result<bool, String> {
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
    let shown: Vec<Toplevel> = state.handles.iter().map(|(_, t)| t.clone()).collect();
    let hit = pick(&shown);
    if let Some(h) = hit.and_then(|i| state.handles.get(i)) {
        match what {
            Do::Activate => h.0.activate(&seat),
            Do::Minimize => h.0.set_minimized(),
            Do::Maximize => h.0.set_maximized(),
        }
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
            state.handles.push((toplevel, Toplevel::default()));
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
                if let Some((_, t)) = state.handles.iter_mut().find(|(x, _)| x == h) {
                    t.app_id = app_id;
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => {
                if let Some((_, t)) = state.handles.iter_mut().find(|(x, _)| x == h) {
                    t.title = title;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_listed_is_found_by_its_app_id_and_title() {
        let t = |app_id: &str, title: &str| Toplevel {
            app_id: app_id.into(),
            title: title.into(),
        };
        let ws = [
            t("reprise.notepad", "a.txt - Notepad"),
            t("reprise.explorer", "b.txt - Notepad"),
            t("reprise.notepad", "b.txt - Notepad"),
        ];
        let listed = |title: &str| reprisebus::tray::AppWindow {
            app_id: "reprise.notepad".into(),
            title: title.into(),
            ..Default::default()
        };
        assert_eq!(the_one(&ws, &listed("b.txt - Notepad")), Some(2));
        assert_eq!(
            the_one(&ws, &listed("c.txt - Notepad")),
            None,
            "a title changed since the sample is no window to close"
        );
    }
}
