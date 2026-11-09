//! logind's view of a process and its user, read from the files `sd-login` reads.

/// The logind session a `/proc/<pid>/cgroup` listing puts the process in: the first unit below
/// the slices being `session-<id>.scope`, as `sd_pid_get_session` reads it.
pub fn session_of_cgroup(cgroup: &str) -> Option<&str> {
    let path = cgroup.lines().find_map(|l| l.strip_prefix("0::"))?;
    let unit = path
        .split('/')
        .find(|c| !c.is_empty() && !c.ends_with(".slice"))?;
    unit.strip_prefix("session-")?.strip_suffix(".scope")
}

/// `ONLINE_SESSIONS=` of logind's record of a user (`/run/systemd/users/<uid>`): the user's
/// sessions that are not closing.
pub fn online_sessions(record: &str) -> Vec<&str> {
    (record.lines())
        .find_map(|l| l.strip_prefix("ONLINE_SESSIONS="))
        .map_or_else(Vec::new, |v| v.split_whitespace().collect())
}

/// `DISPLAY=` of logind's record of a user (`/run/systemd/users/<uid>`), which
/// `sd_uid_get_display` reads.
pub fn display_of_user(record: &str) -> Option<&str> {
    let display = record.lines().find_map(|l| l.strip_prefix("DISPLAY="))?;
    (!display.is_empty()).then_some(display)
}

/// `SERVICE=` of logind's record of a session (`/run/systemd/sessions/<id>`): the PAM service
/// that opened it.
pub fn service_of_session(record: &str) -> Option<&str> {
    let service = record.lines().find_map(|l| l.strip_prefix("SERVICE="))?;
    (!service.is_empty()).then_some(service)
}

/// logind's record of session `id`; empty when there is none.
pub fn session_record(id: &str) -> String {
    std::fs::read_to_string(format!("/run/systemd/sessions/{id}")).unwrap_or_default()
}

/// This process's `/proc/self/cgroup` listing; empty when unreadable.
pub fn own_cgroup() -> String {
    std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default()
}

/// logind's record of the user running this process; empty when there is none.
pub fn own_user_record() -> String {
    // SAFETY: getuid cannot fail and touches no memory.
    let uid = unsafe { libc::getuid() };
    std::fs::read_to_string(format!("/run/systemd/users/{uid}")).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_in_a_session_scope_has_that_session_and_one_under_the_user_manager_none() {
        let session = "0::/user.slice/user-1000.slice/session-3.scope\n";
        assert_eq!(session_of_cgroup(session), Some("3"));
        let below = "0::/user.slice/user-1000.slice/session-c2.scope/payload\n";
        assert_eq!(session_of_cgroup(below), Some("c2"));
        let v1_first = "12:pids:/user.slice\n0::/user.slice/user-1000.slice/session-7.scope\n";
        assert_eq!(session_of_cgroup(v1_first), Some("7"));
        let app =
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-reprise-x-1.scope";
        assert_eq!(session_of_cgroup(app), None);
        assert_eq!(session_of_cgroup("0::/init.scope\n"), None);
        assert_eq!(session_of_cgroup("0::/\n"), None);
    }

    #[test]
    fn the_display_session_is_the_users_logind_record_display_key() {
        let record = "NAME=ann\nSTATE=active\nDISPLAY=3\nSESSIONS=5 3\n";
        assert_eq!(display_of_user(record), Some("3"));
        assert_eq!(display_of_user("NAME=ann\nDISPLAY=\n"), None);
        assert_eq!(display_of_user("NAME=ann\nSESSIONS=5\n"), None);
    }

    #[test]
    fn the_online_sessions_are_the_users_logind_record_online_sessions_key() {
        let record = "NAME=ann\nSESSIONS=11 10 3\nACTIVE_SESSIONS=11 3\nONLINE_SESSIONS=11 3\n";
        assert_eq!(online_sessions(record), ["11", "3"]);
        assert!(online_sessions("NAME=ann\nONLINE_SESSIONS=\n").is_empty());
        assert!(online_sessions("NAME=ann\n").is_empty());
    }
}
