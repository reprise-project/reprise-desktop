//! reprise-login-rdp: the Remote Desktop front of reprise-login.

use std::process::ExitCode;

fn main() -> ExitCode {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    log::error!("not built yet");
    ExitCode::FAILURE
}
