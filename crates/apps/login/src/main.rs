use std::os::fd::FromRawFd;
use std::os::unix::net::UnixStream;

use reprise_login::worker::{Mode, worker_main};

const USAGE: &str = "\
Usage: reprise-login

Reprise's login manager. It runs as root from reprise-login.service.
";

/// The worker socket the daemon passes to `--worker`.
const WORKER_FD: i32 = 3;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let [_, flag, mode, user] = args.as_slice()
        && flag == "--worker"
    {
        std::process::exit(worker(mode, user));
    }
    print!("{USAGE}");
}

fn worker(mode: &str, user: &str) -> i32 {
    let Some(mode) = Mode::parse(mode) else {
        eprintln!("reprise-login: unknown worker mode {mode:?}");
        return 2;
    };
    // SAFETY: fstat fills a local struct; F_SETFD only changes fd 3's flags.
    let is_socket = unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        libc::fstat(WORKER_FD, &mut st) == 0
            && st.st_mode & libc::S_IFMT == libc::S_IFSOCK
            && libc::fcntl(WORKER_FD, libc::F_SETFD, libc::FD_CLOEXEC) == 0
    };
    if !is_socket {
        eprintln!("reprise-login: --worker needs its socket on fd {WORKER_FD}");
        return 2;
    }
    // SAFETY: fd 3 is an open socket nothing else in this process owns.
    let sock = unsafe { UnixStream::from_raw_fd(WORKER_FD) };
    worker_main(mode, user, sock)
}
