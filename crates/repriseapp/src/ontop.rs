//! Reprise's private `reprise_on_top_v1` protocol, generated from the same XML the compositor
//! builds its side from: the main window kept above the others as the program asks
//! ([`Extended::on_top`](crate::Extended::on_top)), and each change made elsewhere as
//! [`Event::OnTop`](crate::Event::OnTop).
#![allow(
    non_upper_case_globals,
    non_camel_case_types,
    unused_imports,
    dead_code,
    clippy::all
)]

// The scanner macros open the XML with File::open, which leaves cargo no
// dependency edge, so this keeps an edit to the protocol rebuilding us.
const _: &[u8] = include_bytes!("../../../protocols/reprise-on-top-v1.xml");

pub mod __interfaces {
    use smithay_client_toolkit::reexports::protocols::xdg::shell::client::__interfaces::*;
    use wayland_client::backend as wayland_backend;
    use wayland_client::protocol::__interfaces::*;
    wayland_scanner::generate_interfaces!("../../protocols/reprise-on-top-v1.xml");
}
use self::__interfaces::*;
use smithay_client_toolkit::reexports::protocols::xdg::shell::client::*;
use wayland_client;
use wayland_client::protocol::*;
wayland_scanner::generate_client_code!("../../protocols/reprise-on-top-v1.xml");
