//! The open-file limit of the compositor process.

/// The `RLIMIT_NOFILE` the compositor started with, before [`raise`].
static ORIGINAL: std::sync::OnceLock<libc::rlimit> = std::sync::OnceLock::new();

/// Raise the soft `RLIMIT_NOFILE` to the hard limit and return the new soft limit. The default
/// soft limit of 1024 runs out with a few dozen clients, each holding sockets, buffers and pipes.
/// Programs the compositor starts get the original back ([`original`]): `select()` cannot take a
/// descriptor past 1023.
pub fn raise() -> std::io::Result<libc::rlim_t> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `limit` is a valid rlimit for getrlimit to fill and setrlimit to read.
    unsafe {
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let soft = limit.rlim_cur;
        let _ = ORIGINAL.set(limit);
        if soft < limit.rlim_max {
            limit.rlim_cur = limit.rlim_max;
            if libc::setrlimit(libc::RLIMIT_NOFILE, &limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        log::debug!("open-file limit {soft} -> {} (hard)", limit.rlim_cur);
    }
    Ok(limit.rlim_cur)
}

/// The limit [`raise`] found, None before it ran.
pub fn original() -> Option<libc::rlimit> {
    ORIGINAL.get().copied()
}

/// Give process `pid`, just started, the limit the compositor started with.
pub fn restore(pid: u32) {
    let Some(limit) = original() else { return };
    // SAFETY: `limit` is a valid rlimit and the old-limit out-param may be null.
    let r = unsafe {
        libc::prlimit(
            pid as libc::pid_t,
            libc::RLIMIT_NOFILE,
            &limit,
            std::ptr::null_mut(),
        )
    };
    if r != 0 {
        log::debug!(
            "open-file limit of {pid}: {}",
            std::io::Error::last_os_error()
        );
    }
}

/// Put the compositor's own limit back to the original, before it execs itself again so the new
/// image's [`raise`] finds the same original.
pub fn lower() {
    if let Some(limit) = original() {
        // SAFETY: `limit` is a valid rlimit.
        unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) };
    }
}

#[cfg(test)]
pub(crate) fn set_original(limit: libc::rlimit) {
    let _ = ORIGINAL.set(limit);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nofile() -> libc::rlimit {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: as in `raise`.
        let r = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) };
        assert_eq!(r, 0);
        limit
    }

    #[test]
    fn raise_lifts_the_soft_limit_to_the_hard_one_and_repeats_harmlessly() {
        // Lowering the limit would starve the other tests of descriptors: run alone.
        const ALONE: &str = "REPRISE_FDLIMIT_TEST_ALONE";
        if std::env::var_os(ALONE).is_none() {
            let name =
                "fdlimit::tests::raise_lifts_the_soft_limit_to_the_hard_one_and_repeats_harmlessly";
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", name, "--quiet", "--test-threads=1"])
                .env(ALONE, "1")
                .status()
                .unwrap();
            assert!(status.success(), "the test passes in a process of its own");
            return;
        }
        let hard = nofile().rlim_max;
        let low = libc::rlimit {
            rlim_cur: 64.min(hard),
            rlim_max: hard,
        };
        // SAFETY: as in `raise`.
        let r = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &low) };
        assert_eq!(r, 0);
        assert_eq!(
            raise().unwrap(),
            hard,
            "the soft limit rises to the hard one"
        );
        assert_eq!(nofile().rlim_cur, hard);
        assert_eq!(
            raise().unwrap(),
            hard,
            "a second raise keeps the soft limit at the hard one"
        );
        assert_eq!(nofile().rlim_max, hard, "the hard limit stays");
    }
}
