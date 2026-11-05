//! `reprise_on_top_v1`: a client keeps one of its toplevels above the other windows, and hears
//! each change the window menu, a key or a rule makes. Glue only: `Window::on_top` holds it.

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel;
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use crate::compositor::State;
use crate::window::WindowId;

use proto::reprise_on_top_v1::{self, RepriseOnTopV1};

#[allow(
    dead_code,
    non_camel_case_types,
    non_upper_case_globals,
    unused_imports,
    clippy::all
)]
pub mod proto {
    // The scanner macros open the XML with File::open, which leaves cargo no
    // dependency edge; this keeps an edit to the protocol rebuilding us.
    const _: &[u8] = include_bytes!("../protocols/reprise-on-top-v1.xml");

    use smithay::reexports::wayland_protocols::xdg::shell::server::*;
    use smithay::reexports::wayland_server;
    use smithay::reexports::wayland_server::protocol::*;

    pub mod __interfaces {
        use smithay::reexports::wayland_protocols::xdg::shell::server::__interfaces::*;
        use smithay::reexports::wayland_server::backend as wayland_backend;
        use smithay::reexports::wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/reprise-on-top-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("protocols/reprise-on-top-v1.xml");
}

const VERSION: u32 = 1;

pub fn advertise(dh: &DisplayHandle) {
    dh.create_global::<State, RepriseOnTopV1, _>(VERSION, OnTopGlobal);
}

pub struct OnTopGlobal;

pub struct OnTopData;

impl GlobalDispatch2<RepriseOnTopV1, State> for OnTopGlobal {
    fn bind(
        &self,
        state: &mut State,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<RepriseOnTopV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        let m = data_init.init(resource, OnTopData);
        state.on_top_managers.retain(Resource::is_alive);
        state.on_top_managers.push(m);
    }
}

impl Dispatch2<RepriseOnTopV1, State> for OnTopData {
    fn request(
        &self,
        state: &mut State,
        client: &Client,
        _resource: &RepriseOnTopV1,
        request: reprise_on_top_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            reprise_on_top_v1::Request::Destroy => {}
            reprise_on_top_v1::Request::Set { toplevel, on_top } => {
                if toplevel.client().map(|c| c.id()) != Some(client.id()) {
                    return;
                }
                let Some(w) = state.window_of_toplevel(&toplevel) else {
                    return;
                };
                let on = on_top != 0;
                match state.win_mut(w) {
                    Some(win) if !win.mapped => win.on_top = on,
                    Some(_) => state.set_on_top(w, on),
                    None => {}
                }
            }
        }
    }
}

impl State {
    fn window_of_toplevel(&self, t: &XdgToplevel) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|w| w.xdg().is_some_and(|x| x.xdg_toplevel() == t))
            .map(|w| w.id)
    }

    /// Tell `w`'s client whether `w` is on top, through each manager it bound.
    pub fn on_top_changed(&mut self, w: WindowId) {
        let Some(win) = self.win(w) else {
            return;
        };
        let (Some(t), on) = (win.xdg(), win.on_top) else {
            return;
        };
        let toplevel = t.xdg_toplevel().clone();
        let client = toplevel.client().map(|c| c.id());
        self.on_top_managers.retain(Resource::is_alive);
        for m in &self.on_top_managers {
            if m.client().map(|c| c.id()) == client {
                m.changed(&toplevel, u32::from(on));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_client_keeps_its_window_on_top_and_hears_the_window_menu_change_it() {
        let mut h = crate::harness::Harness::new();
        h.output("o", 800, 600);
        let mut c = h.connect();
        let m = c.bind_on_top();
        let t = h.map_toplevel(&mut c, "app", 200, 100);
        h.roundtrip(&mut c);
        assert_eq!(c.side.on_top, [false], "the map tells the state");
        m.set(&c.side.shells[t.shell].toplevel, 1);
        h.roundtrip(&mut c);
        assert!(
            h.state.win(t.id).is_some_and(|w| w.on_top),
            "the ask puts it on top"
        );
        assert_eq!(c.side.on_top, [false, true]);
        h.state.set_on_top(t.id, false);
        h.roundtrip(&mut c);
        assert_eq!(
            c.side.on_top,
            [false, true, false],
            "the window menu's change is heard"
        );
    }

    #[test]
    fn an_ask_before_the_map_holds_from_the_map_on() {
        let mut h = crate::harness::Harness::new();
        h.output("o", 800, 600);
        let mut c = h.connect();
        let m = c.bind_on_top();
        let t = h.map_toplevel_after(&mut c, "app", 200, 100, |hs, c, t| {
            m.set(&c.side.shells[t.shell].toplevel, 1);
            hs.roundtrip(c);
        });
        h.roundtrip(&mut c);
        assert!(h.state.win(t.id).is_some_and(|w| w.on_top));
        assert_eq!(c.side.on_top, [true]);
    }
}
