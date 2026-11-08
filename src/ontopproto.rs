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

    fn can_view(&self, client: &Client) -> bool {
        crate::sandbox::client_may_see(client, "reprise_on_top_v1")
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
    use crate::window::WindowId;
    use reprisescene::NodeId;

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

    #[test]
    fn a_sandboxed_client_is_not_offered_the_on_top_global() {
        let mut h = crate::harness::Harness::new();
        let offered = |c: &crate::harness::TestClient| c.offered("reprise_on_top_v1");
        let mut host = h.connect();
        h.roundtrip(&mut host);
        assert!(offered(&host), "a program outside a sandbox gets it");
        let mut boxed = h.connect_sandboxed();
        h.roundtrip(&mut boxed);
        assert!(!offered(&boxed), "a Flatpak app does not");
        let mut ctx = h.connect_in_flatpak("org.example.App");
        h.roundtrip(&mut ctx);
        assert!(!offered(&ctx), "nor one on a security context's socket");
    }

    #[test]
    fn a_consent_prompt_stays_above_an_on_top_window() {
        let mut h = crate::harness::Harness::new();
        h.output("o", 800, 600);
        let mut c = h.connect();
        let m = c.bind_on_top();
        let app = h.map_toplevel(&mut c, "app", 400, 300);
        m.set(&c.side.shells[app.shell].toplevel, 1);
        h.roundtrip(&mut c);
        let mut p = h.connect_as(crate::mapfocus::TEST_PROMPT_PID);
        let prompt = h.map_toplevel(&mut p, crate::mapfocus::CONSENT_APP_ID, 300, 150);
        let password = h.map_toplevel(&mut p, "reprise.polkit", 300, 150);
        h.state.focus_window(app.id);
        h.roundtrip(&mut c);
        let layer = |h: &crate::harness::Harness, w: WindowId| {
            h.state.win(w).and_then(|x| h.state.scene.parent(x.tree))
        };
        let l = &h.state.layers;
        assert_eq!(layer(&h, app.id), Some(l.on_top));
        assert_eq!(layer(&h, prompt.id), Some(l.prompts), "the prompt");
        assert_eq!(
            layer(&h, password.id),
            Some(l.windows),
            "a password box stacks like any window"
        );
        let root = h.state.scene.children(h.state.scene.root());
        let at = |t| root.iter().position(|&n| n == NodeId::from(t));
        assert!(
            at(l.prompts) > at(l.on_top),
            "prompts sit over on-top windows"
        );
    }

    #[test]
    fn a_sandboxed_client_claiming_the_portals_pid_draws_no_consent_prompt() {
        let mut h = crate::harness::Harness::new();
        h.output("o", 800, 600);
        let mut c = h.connect();
        let app = h.map_toplevel(&mut c, "app", 400, 300);
        let mut boxed = h.connect_sandboxed_as(crate::mapfocus::TEST_PROMPT_PID);
        let fake = h.map_toplevel(&mut boxed, crate::mapfocus::CONSENT_APP_ID, 300, 150);
        let w = h.state.win(fake.id).unwrap();
        assert!(!w.consent);
        assert_eq!(h.state.scene.parent(w.tree), Some(h.state.layers.windows));
        assert_eq!(
            h.state.client_pid(fake.id),
            Some(crate::mapfocus::TEST_PROMPT_PID)
        );
        assert_eq!(h.state.prompt_pid(fake.id), None, "no prompt rights");
        assert!(h.state.prompt_pid(app.id).is_some());
    }
}
