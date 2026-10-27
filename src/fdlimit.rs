//! The open-file limit of the compositor process.

/// Raise the soft `RLIMIT_NOFILE` to the hard limit and return the new soft limit. The default
/// soft limit of 1024 runs out with a few dozen clients, each holding sockets, buffers and pipes.
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
