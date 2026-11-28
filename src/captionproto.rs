//! `reprise_caption_v1`: a client asks for a toplevel's caption without the window icon and the
//! title. Glue only: `Window::bare_caption` holds it, `draw_strips` reads it.

use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource, WEnum};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use crate::compositor::State;

use proto::reprise_caption_v1::{self, RepriseCaptionV1, Style};

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
    const _: &[u8] = include_bytes!("../protocols/reprise-caption-v1.xml");

    use smithay::reexports::wayland_protocols::xdg::shell::server::*;
    use smithay::reexports::wayland_server;
    use smithay::reexports::wayland_server::protocol::*;

    pub mod __interfaces {
        use smithay::reexports::wayland_protocols::xdg::shell::server::__interfaces::*;
        use smithay::reexports::wayland_server::backend as wayland_backend;
        use smithay::reexports::wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/reprise-caption-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("protocols/reprise-caption-v1.xml");
}

const VERSION: u32 = 1;

pub fn advertise(dh: &DisplayHandle) {
    dh.create_global::<State, RepriseCaptionV1, _>(VERSION, CaptionGlobal);
}

pub struct CaptionGlobal;

pub struct CaptionData;

impl GlobalDispatch2<RepriseCaptionV1, State> for CaptionGlobal {
    fn bind(
        &self,
        _state: &mut State,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<RepriseCaptionV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, CaptionData);
    }

    fn can_view(&self, client: &Client) -> bool {
        crate::sandbox::client_may_see(client, "reprise_caption_v1")
    }
}

impl Dispatch2<RepriseCaptionV1, State> for CaptionData {
    fn request(
        &self,
        state: &mut State,
        client: &Client,
        _resource: &RepriseCaptionV1,
        request: reprise_caption_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            reprise_caption_v1::Request::Destroy => {}
            reprise_caption_v1::Request::Set { toplevel, style } => {
                if toplevel.client().map(|c| c.id()) != Some(client.id()) {
                    return;
                }
                let Some(w) = state.window_of_toplevel(&toplevel) else {
                    return;
                };
                let bare = style == WEnum::Value(Style::Bare);
                if let Some(win) = state.win_mut(w)
                    && win.bare_caption != bare
                {
                    win.bare_caption = bare;
                    state.redraw_caption(w);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::harness::Harness;
    use crate::harness::caption_client::reprise_caption_v1::{RepriseCaptionV1, Style};

    #[test]
    fn a_bare_caption_draws_no_title_and_follows_each_set() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let caption: RepriseCaptionV1 = c.bind("reprise_caption_v1", 1);
        let t = h.map_toplevel_after(&mut c, "ed", 300, 200, |h, c, t| {
            let top = &c.side.shells[t.shell].toplevel;
            top.set_title("Notes".into());
            caption.set(top, Style::Bare);
            h.roundtrip(c);
        });
        let top = c.side.shells[t.shell].toplevel.clone();
        let bar = |h: &Harness| {
            let frame = h.state.win(t.id).expect("the window").frame;
            h.state
                .scene
                .buffer_canvas(frame[0])
                .expect("a drawn caption")
        };
        assert!(
            h.state.win(t.id).unwrap().bare_caption,
            "asked before the map"
        );
        let bare = bar(&h);
        top.set_title("A much longer title".into());
        h.roundtrip(&mut c);
        assert!(bar(&h) == bare, "the title is not drawn");
        assert_eq!(h.state.win(t.id).unwrap().title(), "A much longer title");

        caption.set(&top, Style::Full);
        h.roundtrip(&mut c);
        assert!(!h.state.win(t.id).unwrap().bare_caption);
        assert!(bar(&h) != bare, "full draws the icon and the title again");
        caption.set(&top, Style::Bare);
        h.roundtrip(&mut c);
        assert!(bar(&h) == bare, "bare again");
    }

    #[test]
    fn a_bare_captions_icon_place_moves_the_window_like_the_rest() {
        use reprisekit::frame::FrameHit;
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let caption: RepriseCaptionV1 = c.bind("reprise_caption_v1", 1);
        let t = h.map_toplevel(&mut c, "ed", 300, 200);
        let hit = |h: &Harness| {
            let win = h.state.win(t.id).expect("the window");
            let r = win.geom.caption;
            let (x, y) = (win.x + r.x + 4, win.y + r.y + r.height / 2);
            win.hit_at(f64::from(x), f64::from(y), &h.state.config.theme.metrics)
        };
        assert_eq!(hit(&h), FrameHit::Icon, "framed, it is the icon");
        caption.set(&c.side.shells[t.shell].toplevel, Style::Bare);
        h.roundtrip(&mut c);
        assert_eq!(hit(&h), FrameHit::Title, "bare, it is caption");
    }

    #[test]
    fn a_sandboxed_client_is_not_offered_the_global() {
        let mut h = Harness::new();
        let offered = |c: &crate::harness::TestClient| c.offered("reprise_caption_v1");
        let mut host = h.connect();
        h.roundtrip(&mut host);
        assert!(offered(&host), "a program outside a sandbox gets it");
        let mut boxed = h.connect_sandboxed();
        h.roundtrip(&mut boxed);
        assert!(!offered(&boxed), "a Flatpak app does not");
    }
}
