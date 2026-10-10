//! Client bindings for Reprise's private `reprise_input_v1` protocol,
//! generated from the same XML the compositor builds its statics from.
#![allow(
    non_upper_case_globals,
    non_camel_case_types,
    unused_imports,
    dead_code,
    clippy::all
)]

// The scanner macros open the XML with File::open, which leaves cargo no
// dependency edge, so this keeps an edit to the protocol rebuilding us.
const _: &[u8] = include_bytes!("../../../protocols/reprise-input-v1.xml");

pub mod __interfaces {
    use wayland_client::protocol::__interfaces::*;
    wayland_scanner::generate_interfaces!("../../protocols/reprise-input-v1.xml");
}
use self::__interfaces::*;
use wayland_client;
use wayland_client::protocol::*;
wayland_scanner::generate_client_code!("../../protocols/reprise-input-v1.xml");
