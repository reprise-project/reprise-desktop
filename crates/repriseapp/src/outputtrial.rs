//! Reprise's private `reprise_output_trial_v1` protocol, generated from the same XML the
//! compositor builds its side from: a configuration the compositor puts back unless the program
//! keeps it ([`HeadRequests::configure_on_trial`](crate::heads::HeadRequests::configure_on_trial)).
#![allow(
    non_upper_case_globals,
    non_camel_case_types,
    unused_imports,
    dead_code,
    clippy::all
)]

// The scanner macros open the XML with File::open, which leaves cargo no
// dependency edge, so this keeps an edit to the protocol rebuilding us.
const _: &[u8] = include_bytes!("../../../protocols/reprise-output-trial-v1.xml");

pub mod __interfaces {
    use smithay_client_toolkit::reexports::protocols_wlr::output_management::v1::client::__interfaces::*;
    use wayland_client::backend as wayland_backend;
    wayland_scanner::generate_interfaces!("../../protocols/reprise-output-trial-v1.xml");
}
use self::__interfaces::*;
use smithay_client_toolkit::reexports::protocols_wlr::output_management::v1::client::*;
use wayland_client;
wayland_scanner::generate_client_code!("../../protocols/reprise-output-trial-v1.xml");
