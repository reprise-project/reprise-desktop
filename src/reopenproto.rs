//! `reprise_reopen_v1`: a client names the argument each of its toplevels starts again with at
//! session restore. Glue only: `Window::reopen` holds it, `session` keeps and uses it.

use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use crate::compositor::State;

use proto::reprise_reopen_v1::{self, RepriseReopenV1};

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
    const _: &[u8] = include_bytes!("../protocols/reprise-reopen-v1.xml");

    use smithay::reexports::wayland_protocols::xdg::shell::server::*;
    use smithay::reexports::wayland_server;
    use smithay::reexports::wayland_server::protocol::*;

    pub mod __interfaces {
        use smithay::reexports::wayland_protocols::xdg::shell::server::__interfaces::*;
        use smithay::reexports::wayland_server::backend as wayland_backend;
        use smithay::reexports::wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/reprise-reopen-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("protocols/reprise-reopen-v1.xml");
}

const VERSION: u32 = 1;

pub fn advertise(dh: &DisplayHandle) {
    dh.create_global::<State, RepriseReopenV1, _>(VERSION, ReopenGlobal);
}

pub struct ReopenGlobal;

pub struct ReopenData;

impl GlobalDispatch2<RepriseReopenV1, State> for ReopenGlobal {
    fn bind(
        &self,
        _state: &mut State,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<RepriseReopenV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ReopenData);
    }

    fn can_view(&self, client: &Client) -> bool {
        crate::sandbox::client_may_see(client, "reprise_reopen_v1")
    }
}

/// The longest argument a window may name, in bytes.
const ARGUMENT_MAX: usize = 1024;

/// The argument a `set` names: none for null, empty, over [`ARGUMENT_MAX`] or one holding a
/// control character.
fn argument(a: Option<String>) -> Option<String> {
    a.filter(|a| !a.is_empty() && a.len() <= ARGUMENT_MAX && !a.contains(char::is_control))
}

impl Dispatch2<RepriseReopenV1, State> for ReopenData {
    fn request(
        &self,
        state: &mut State,
        client: &Client,
        _resource: &RepriseReopenV1,
        request: reprise_reopen_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            reprise_reopen_v1::Request::Destroy => {}
            reprise_reopen_v1::Request::Set {
                toplevel,
                argument: a,
            } => {
                if toplevel.client().map(|c| c.id()) != Some(client.id()) {
                    return;
                }
                let Some(w) = state.window_of_toplevel(&toplevel) else {
                    return;
                };
                let a = argument(a);
                if let Some(win) = state.win_mut(w)
                    && win.reopen != a
                {
                    win.reopen = a;
                    state.session.owe();
                    state.replan_unmapped(w);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::harness::Harness;

    #[test]
    fn a_window_keeps_the_argument_its_client_names_and_drops_a_bad_one() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let reopen = c.bind_reopen();
        let t = h.map_toplevel_after(&mut c, "ed", 300, 200, |h, c, t| {
            reopen.set(&c.side.shells[t.shell].toplevel, Some("/d/a.txt".into()));
            h.roundtrip(c);
        });
        let arg = |h: &Harness| h.state.win(t.id).unwrap().reopen.clone();
        assert_eq!(arg(&h).as_deref(), Some("/d/a.txt"), "named before the map");
        for bad in [None, Some(""), Some("a\nb")] {
            reopen.set(&c.side.shells[t.shell].toplevel, Some("/d/a.txt".into()));
            reopen.set(&c.side.shells[t.shell].toplevel, bad.map(String::from));
            h.roundtrip(&mut c);
            assert_eq!(arg(&h), None, "{bad:?} names none");
        }
    }

    #[test]
    fn an_argument_over_1024_bytes_names_none() {
        let longest = "/".repeat(super::ARGUMENT_MAX);
        assert_eq!(super::argument(Some(longest.clone())), Some(longest));
        assert_eq!(super::argument(Some("/".repeat(1025))), None);
    }

    #[test]
    fn a_sandboxed_client_is_not_offered_the_global() {
        let mut h = Harness::new();
        let offered = |c: &crate::harness::TestClient| c.offered("reprise_reopen_v1");
        let mut host = h.connect();
        h.roundtrip(&mut host);
        assert!(offered(&host), "a program outside a sandbox gets it");
        let mut boxed = h.connect_sandboxed();
        h.roundtrip(&mut boxed);
        assert!(!offered(&boxed), "a Flatpak app does not");
    }
}
