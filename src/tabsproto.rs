//! `reprise_tabs_v1`: a client asks for one of its toplevels to join the caption tab group of
//! another of its own. Glue only: the join waits in `Window::tab_leader` for `tab_on_map`.

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel;
use smithay::reexports::wayland_server::{
    Client, DataInit, DisplayHandle, New, Resource, backend::ClientId,
};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use crate::compositor::State;
use crate::window::WindowId;

use proto::reprise_tabs_v1::{self, RepriseTabsV1};

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
    const _: &[u8] = include_bytes!("../protocols/reprise-tabs-v1.xml");

    use smithay::reexports::wayland_protocols::xdg::shell::server::*;
    use smithay::reexports::wayland_server;
    use smithay::reexports::wayland_server::protocol::*;

    pub mod __interfaces {
        use smithay::reexports::wayland_protocols::xdg::shell::server::__interfaces::*;
        use smithay::reexports::wayland_server::backend as wayland_backend;
        use smithay::reexports::wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/reprise-tabs-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("protocols/reprise-tabs-v1.xml");
}

const VERSION: u32 = 1;

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<State, RepriseTabsV1, _>(VERSION, TabsGlobal);
}

pub struct TabsGlobal;

pub struct TabsData;

impl GlobalDispatch2<RepriseTabsV1, State> for TabsGlobal {
    fn bind(
        &self,
        _state: &mut State,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<RepriseTabsV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, TabsData);
    }
}

impl Dispatch2<RepriseTabsV1, State> for TabsData {
    fn request(
        &self,
        state: &mut State,
        client: &Client,
        _resource: &RepriseTabsV1,
        request: reprise_tabs_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            reprise_tabs_v1::Request::Destroy => {}
            reprise_tabs_v1::Request::Join { toplevel, leader } => {
                let (Some(w), Some(l)) = (state.window_of(&toplevel), state.window_of(&leader))
                else {
                    return;
                };
                if !joinable(client.id(), owner(&toplevel), owner(&leader), w, l) {
                    log::warn!("reprise_tabs_v1.join: {w:?} cannot join {l:?}");
                    return;
                }
                state.join_leader(w, l);
            }
        }
    }
}

fn owner(t: &XdgToplevel) -> Option<ClientId> {
    t.client().map(|c| c.id())
}

/// Whether the asking client may put `w` (owned by `w_owner`) into the group of `l` (owned by
/// `l_owner`): both are its own, and they are two windows.
fn joinable<C: PartialEq, W: PartialEq>(
    asker: C,
    w_owner: Option<C>,
    l_owner: Option<C>,
    w: W,
    l: W,
) -> bool {
    w != l && w_owner.as_ref() == Some(&asker) && l_owner.as_ref() == Some(&asker)
}

impl State {
    fn window_of(&self, t: &XdgToplevel) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|w| w.xdg().is_some_and(|x| x.xdg_toplevel() == t))
            .map(|w| w.id)
    }
}

#[cfg(test)]
mod tests {
    use super::joinable;

    #[test]
    fn only_two_windows_of_the_asking_client_join() {
        assert!(joinable(1, Some(1), Some(1), 10, 11));
        assert!(
            !joinable(1, Some(1), Some(1), 10, 10),
            "a window joins no group of its own"
        );
        assert!(
            !joinable(1, Some(2), Some(1), 10, 11),
            "the toplevel is another client's"
        );
        assert!(
            !joinable(1, Some(1), Some(2), 10, 11),
            "the leader is another client's"
        );
        assert!(!joinable(1, None, Some(1), 10, 11), "the toplevel is gone");
    }
}
