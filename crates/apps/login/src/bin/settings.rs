//! `pkexec reprise-login-settings`: replaces `MACHINE_CONFIG` with the TOML on stdin. Exit 0 is
//! applied, 2 is refused with one reason per line on stderr, 1 is any other failure.

use std::ffi::CString;
use std::io::Read as _;
use std::path::Path;
use std::process::{Command, ExitCode};

use reprise_login::machinewrite::{self, Error, MAX_INPUT};
use repriselogin::machine::MACHINE_CONFIG;

const UNIT: &str = "reprise-login.service";
const SYSTEMCTL: &str = "/usr/bin/systemctl";

/// Prints the library's warnings, such as a directory sync failing after the rename, to stderr.
struct Stderr;

impl log::Log for Stderr {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record) {
        match record.level() {
            log::Level::Error => eprintln!("error: {}", record.args()),
            log::Level::Warn => eprintln!("warning: {}", record.args()),
            _ => {}
        }
    }

    fn flush(&self) {}
}

fn main() -> ExitCode {
    static STDERR: Stderr = Stderr;
    if log::set_logger(&STDERR).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
    }
    // SAFETY: umask has no preconditions. pkexec passes on the caller's umask.
    unsafe { libc::umask(0o022) };
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("reprise-login-settings runs as root, through pkexec");
        return ExitCode::from(1);
    }
    let mut input = Vec::new();
    if let Err(e) = std::io::stdin()
        .take(MAX_INPUT as u64 + 1)
        .read_to_end(&mut input)
    {
        eprintln!("reading the settings: {e}");
        return ExitCode::from(1);
    }
    if input.len() > MAX_INPUT {
        eprintln!("the settings are too long");
        return ExitCode::from(2);
    }
    let Ok(input) = String::from_utf8(input) else {
        eprintln!("the settings are not UTF-8");
        return ExitCode::from(2);
    };
    match machinewrite::apply(&input, Path::new(MACHINE_CONFIG)) {
        Ok(()) => {}
        Err(Error::Invalid(warnings)) => {
            for w in warnings {
                eprintln!("{}", machinewrite::one_line(&w));
            }
            return ExitCode::from(2);
        }
        Err(Error::Io(e)) => {
            eprintln!("{MACHINE_CONFIG}: {e}");
            return ExitCode::from(1);
        }
    }
    let who = match std::env::var("PKEXEC_UID") {
        Ok(uid) => format!("uid {uid}"),
        Err(_) => "root, without pkexec,".into(),
    };
    journal(&format!(
        "{who} changed the machine settings in {MACHINE_CONFIG}"
    ));
    if active() {
        match Command::new(SYSTEMCTL).args(["reload", UNIT]).status() {
            Ok(s) if s.success() => {}
            Ok(s) => eprintln!("warning: systemctl reload {UNIT}: {s}"),
            Err(e) => eprintln!("warning: systemctl reload {UNIT}: {e}"),
        }
    }
    ExitCode::SUCCESS
}

fn active() -> bool {
    Command::new(SYSTEMCTL)
        .args(["is-active", "--quiet", UNIT])
        .status()
        .is_ok_and(|s| s.success())
}

/// Logs `msg` through syslog, since stderr belongs to the caller under pkexec.
fn journal(msg: &str) {
    let Ok(msg) = CString::new(msg) else {
        return;
    };
    // SAFETY: both strings are NUL-terminated, the ident is static, and "%s" takes one string.
    unsafe {
        libc::openlog(
            c"reprise-login-settings".as_ptr(),
            libc::LOG_PID,
            libc::LOG_AUTHPRIV,
        );
        libc::syslog(libc::LOG_NOTICE, c"%s".as_ptr(), msg.as_ptr());
    }
}
