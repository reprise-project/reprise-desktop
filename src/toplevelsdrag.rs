//! xdg_toplevel_drag_v1: a program's own drag carries one of its windows, as a torn-off browser
//! tab. The drag runs in `dnd`, which calls in here on its start, motion and end.

use smithay::{
    reexports::{
        wayland_protocols::xdg::{
            shell::server::xdg_toplevel::XdgToplevel,
            toplevel_drag::v1::server::{
                xdg_toplevel_drag_manager_v1::{self, XdgToplevelDragManagerV1},
                xdg_toplevel_drag_v1::{self, XdgToplevelDragV1},
            },
        },
        wayland_server::{
            Client, DataInit, DisplayHandle, New, Resource,
            backend::ClientId,
            protocol::{wl_data_source::WlDataSource, wl_surface::WlSurface},
        },
    },
    utils::{Logical, Point},
    wayland::{Dispatch2, GlobalDispatch2},
};

use reprisescene::Input;

use crate::compositor::State;
use crate::window::WindowId;

/// The drag objects clients made, one per data source.
#[derive(Default)]
pub struct ToplevelDrags(Vec<Entry>);

struct Entry {
    resource: XdgToplevelDragV1,
    source: WlDataSource,
    attached: Option<Attached>,
}

#[derive(Clone)]
struct Attached {
    toplevel: XdgToplevel,
    surface: WlSurface,
    /// The pointer's place in the window, from its geometry's origin.
    offset: Point<i32, Logical>,
}

pub fn advertise(dh: &DisplayHandle) {
    dh.create_global::<State, XdgToplevelDragManagerV1, _>(1, Manager);
}

pub struct Manager;

pub struct Drag;

impl State {
    /// The window the running drag carries, with the pointer's place in it.
    fn carried(&self) -> Option<(WindowId, Point<i32, Logical>)> {
        let source = self.drag_source()?.data_source()?.clone();
        let e = self.toplevel_drags.0.iter().find(|e| e.source == source)?;
        let a = e.attached.as_ref().filter(|a| a.toplevel.is_alive())?;
        Some((self.window_for_surface(&a.surface)?, a.offset))
    }

    /// Where the carried `w`'s frame goes with the pointer at `pos`; None for another window.
    pub(crate) fn carried_frame_at(
        &self,
        w: WindowId,
        pos: Point<f64, Logical>,
    ) -> Option<(i32, i32)> {
        let (carried, offset) = self.carried()?;
        let win = self.win(w).filter(|_| carried == w)?;
        let x = pos.x.round() as i32 - offset.x - win.geom.client.x;
        let y = pos.y.round() as i32 - offset.y - win.geom.client.y;
        Some((x, y))
    }

    /// A drag started or a window was attached to it: the window lets the pointer through to
    /// the drop targets beneath for as long as it is carried.
    pub(crate) fn carry_start(&mut self) {
        let Some((w, _)) = self.carried() else {
            return;
        };
        if let Some(s) = self.drag_source() {
            s.set_carries(true);
        }
        if let Some(tree) = self.win(w).map(|w| w.tree) {
            self.scene.set_input(tree, Input::Passthrough);
        }
    }

    /// The drag moved to `pos`: the carried window follows it, unless maximized or fullscreen.
    pub(crate) fn carry_to(&mut self, pos: Point<f64, Logical>) {
        let Some((w, _)) = self.carried() else {
            return;
        };
        let free = self
            .win(w)
            .is_some_and(|w| w.mapped && !w.maximized && !w.fullscreen);
        if free && let Some((x, y)) = self.carried_frame_at(w, pos) {
            self.drag_to(w, x, y);
        }
    }

    /// The drag of `source` ended: the carried window stays where it is and takes the pointer
    /// again.
    pub(crate) fn carry_end(&mut self, source: &WlDataSource) {
        let Some(e) = self
            .toplevel_drags
            .0
            .iter_mut()
            .find(|e| e.source == *source)
        else {
            return;
        };
        if let Some(a) = e.attached.take() {
            self.let_go(source, &a.surface);
        }
    }

    /// `w` unmapped: a drag carrying it lets go of it, and a later attach may carry it again.
    pub(crate) fn carry_unmapped(&mut self, w: WindowId) {
        let Some(surface) = self.win(w).map(|w| w.wl_surface().clone()) else {
            return;
        };
        let held = self.toplevel_drags.0.iter_mut().find_map(|e| {
            e.attached
                .take_if(|a| a.surface == surface)
                .map(|a| (e.source.clone(), a.surface))
        });
        if let Some((source, surface)) = held {
            self.let_go(&source, &surface);
        }
    }

    /// `surface`'s window is carried by `source`'s drag no more; that drag, if still running,
    /// goes on as a plain one.
    fn let_go(&mut self, source: &WlDataSource, surface: &WlSurface) {
        if let Some(s) = self
            .drag_source()
            .filter(|s| s.data_source() == Some(source))
        {
            s.set_carries(false);
        }
        let Some(w) = self.window_for_surface(surface) else {
            return;
        };
        if let Some(tree) = self.win(w).map(|w| w.tree) {
            self.scene.set_input(tree, Input::All);
        }
        self.settle_move(w);
    }

    fn drags_source(&self, source: &WlDataSource) -> bool {
        self.drag_source()
            .is_some_and(|s| s.data_source() == Some(source))
    }
}

impl GlobalDispatch2<XdgToplevelDragManagerV1, State> for Manager {
    fn bind(
        &self,
        _state: &mut State,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<XdgToplevelDragManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, Manager);
    }
}

impl Dispatch2<XdgToplevelDragManagerV1, State> for Manager {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        resource: &XdgToplevelDragManagerV1,
        request: xdg_toplevel_drag_manager_v1::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        let xdg_toplevel_drag_manager_v1::Request::GetXdgToplevelDrag { id, data_source } = request
        else {
            return;
        };
        let drag = data_init.init(id, Drag);
        if state
            .toplevel_drags
            .0
            .iter()
            .any(|e| e.source == data_source)
        {
            resource.post_error(
                xdg_toplevel_drag_manager_v1::Error::InvalidSource,
                "the data source has a toplevel drag already",
            );
            return;
        }
        state.toplevel_drags.0.push(Entry {
            resource: drag,
            source: data_source,
            attached: None,
        });
    }
}

impl Dispatch2<XdgToplevelDragV1, State> for Drag {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        resource: &XdgToplevelDragV1,
        request: xdg_toplevel_drag_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        let Some(i) = state
            .toplevel_drags
            .0
            .iter()
            .position(|e| e.resource == *resource)
        else {
            return;
        };
        match request {
            xdg_toplevel_drag_v1::Request::Attach {
                toplevel,
                x_offset,
                y_offset,
            } => {
                let e = &state.toplevel_drags.0[i];
                if e.attached.as_ref().is_some_and(|a| a.toplevel.is_alive()) {
                    resource.post_error(
                        xdg_toplevel_drag_v1::Error::ToplevelAttached,
                        "a toplevel is attached already",
                    );
                    return;
                }
                let Some(surface) = state
                    .xdg_shell_state
                    .get_toplevel(&toplevel)
                    .map(|t| t.wl_surface().clone())
                else {
                    return;
                };
                let source = e.source.clone();
                state.toplevel_drags.0[i].attached = Some(Attached {
                    toplevel,
                    surface,
                    offset: (x_offset, y_offset).into(),
                });
                if state.drags_source(&source) {
                    state.carry_start();
                }
            }
            xdg_toplevel_drag_v1::Request::Destroy
                if state.drags_source(&state.toplevel_drags.0[i].source) =>
            {
                resource.post_error(
                    xdg_toplevel_drag_v1::Error::OngoingDrag,
                    "the drag has not ended",
                );
            }
            _ => {}
        }
    }

    fn destroyed(&self, state: &mut State, _client: ClientId, resource: &XdgToplevelDragV1) {
        let Some(i) = state
            .toplevel_drags
            .0
            .iter()
            .position(|e| e.resource == *resource)
        else {
            return;
        };
        let e = state.toplevel_drags.0.remove(i);
        if let Some(a) = e.attached {
            state.let_go(&e.source, &a.surface);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::harness::{Harness, TestClient, Toplevel};
    use crate::input::{BTN_LEFT, Hit};
    use crate::window::WindowId;
    use wayland_client::protocol::wl_data_device_manager::DndAction;
    use wayland_protocols::xdg::toplevel_drag::v1::client::xdg_toplevel_drag_manager_v1::XdgToplevelDragManagerV1;

    fn window_at(h: &Harness, x: f64, y: f64) -> Option<WindowId> {
        match h.state.scene_hit((x, y).into()) {
            Hit::Surface { surface, .. } => h.state.window_for_surface(&surface),
            _ => None,
        }
    }

    /// A drag from `browser`'s strip at its client's (50, 20) that carries a window mapped at
    /// 200x150 with the pointer at (30, 10) in it: the strip, the carried window and the press.
    fn tear_off(h: &mut Harness, c: &mut TestClient) -> (Toplevel, Toplevel, (i32, i32)) {
        let strip = h.map_toplevel(c, "browser", 400, 300);
        let (sx, sy) = h.state.win(strip.id).unwrap().client_origin();
        let (px, py) = (sx + 50, sy + 20);
        h.pointer_to(px.into(), py.into());
        h.button(BTN_LEFT, true);
        h.roundtrip(c);
        let source = c.data_source(&["text/plain"], b"tab");
        source.set_actions(DndAction::Move);
        let manager: XdgToplevelDragManagerV1 = c.bind("xdg_toplevel_drag_manager_v1", 1);
        let drag = manager.get_xdg_toplevel_drag(&source, &c.qh(), ());
        let origin = c.side.shells[strip.shell].surface.clone();
        let serial = c.side.button_serial;
        c.data_device()
            .start_drag(Some(&source), &origin, None, serial);
        h.roundtrip(c);
        assert!(h.state.dnd.running(), "the drag started");
        let torn = h.map_toplevel_after(c, "browser", 200, 150, |h, c, t| {
            drag.attach(&c.side.shells[t.shell].toplevel, 30, 10);
            h.roundtrip(c);
        });
        (strip, torn, (px, py))
    }

    #[test]
    fn an_attached_window_maps_and_moves_under_the_pointer_and_lets_drops_through() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let (strip, torn, (px, py)) = tear_off(&mut h, &mut c);
        let at = |h: &Harness| h.state.win(torn.id).unwrap().client_origin();
        assert_eq!(at(&h), (px - 30, py - 10), "it maps under the pointer");
        h.pointer_to(f64::from(px + 100), f64::from(py + 40));
        assert_eq!(at(&h), (px + 70, py + 30), "it follows the pointer");
        assert_eq!(
            window_at(&h, f64::from(px + 100), f64::from(py + 40)),
            Some(strip.id),
            "the drop target is the window beneath"
        );

        h.button(BTN_LEFT, false);
        h.roundtrip(&mut c);
        assert!(!h.state.dnd.running(), "the drop ended the drag");
        assert_eq!(
            c.side.source_ends,
            [true],
            "a drop on no target lands the window: performed, not cancelled"
        );
        assert_eq!(at(&h), (px + 70, py + 30), "it stays where it was dropped");
        assert_eq!(
            window_at(&h, f64::from(px + 100), f64::from(py + 40)),
            Some(torn.id),
            "it takes the pointer again"
        );
    }

    #[test]
    fn a_cancelled_carry_is_cancelled_for_the_source() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        tear_off(&mut h, &mut c);
        h.state.cancel_drag();
        h.roundtrip(&mut c);
        assert_eq!(c.side.source_ends, [false]);
    }
}
