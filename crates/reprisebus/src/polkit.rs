//! reprise-polkit's record of dismissed password prompts, on the session bus under one [`name`]
//! per logind session. A service behind polkit answers the same error for a dismissed prompt and
//! a refused password; a program asks here which one it was.
//! [`allowed_without_prompt`] and [`allowed_without_password`] ask polkit itself whether an action
//! needs a password.

use std::collections::HashMap;

use zbus::Variant;

pub const PATH: &str = "/reprise/polkit/Dismissals";
pub const INTERFACE: &str = "reprise.polkit.Dismissals";

/// The session bus name of the reprise-polkit serving logind session `session`. None for an id
/// with a byte other than a letter or digit, which logind never gives.
pub fn name(session: &str) -> Option<String> {
    let id = !session.is_empty() && session.bytes().all(|b| b.is_ascii_alphanumeric());
    id.then(|| format!("reprise.polkit.session{session}"))
}

/// The logind session polkitd judges a process by, and whose agent shows its prompts: the session
/// its `cgroup` listing is in, else its user's display session from logind's user `record`.
pub fn judged_session<'a>(cgroup: &'a str, record: &'a str) -> Option<&'a str> {
    crate::logind::session_of_cgroup(cgroup).or_else(|| crate::logind::display_of_user(record))
}

/// [`judged_session`] of this process.
pub fn own_judged_session() -> Option<String> {
    let (cgroup, record) = (
        crate::logind::own_cgroup(),
        crate::logind::own_user_record(),
    );
    judged_session(&cgroup, &record).map(String::from)
}

/// [`name`] of the agent polkitd sends the prompts of the process with these files to.
fn agent_name(cgroup: &str, record: &str) -> Option<String> {
    judged_session(cgroup, record).and_then(name)
}

/// Whether the last password prompt polkit opened for this process was
/// dismissed, forgetting it once asked. False when reprise-polkit does not
/// answer. Blocks for at most [`crate::METHOD_TIMEOUT`].
pub fn dismissed() -> bool {
    let (cgroup, record) = (
        crate::logind::own_cgroup(),
        crate::logind::own_user_record(),
    );
    let Some(agent) = agent_name(&cgroup, &record) else {
        log::warn!("no logind session whose reprise-polkit would know of a dismissed prompt");
        return false;
    };
    let answer = crate::session().and_then(|conn| {
        crate::wait(conn.call_method(Some(agent.as_str()), PATH, Some(INTERFACE), "Take", &()))?
            .body()
            .deserialize::<bool>()
    });
    answer.unwrap_or_else(|e| {
        log::warn!("{agent} does not say whether the prompt was dismissed: {e}");
        false
    })
}

/// How a service behind polkit said no.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The password prompt was dismissed: nothing to tell.
    Dismissed,
    /// The password was refused, or polkit allows no one.
    Denied,
    /// The service stopped waiting for the password prompt and answered
    /// `NoReply` (systemd's services wait 25 s), or the caller's own method
    /// timeout ran out first.
    TimedOut,
}

/// What a program says for [`Refusal::TimedOut`].
pub const TIMED_OUT: &str =
    "The password prompt was open too long, so nothing was changed. Try again.";

/// What a program says for [`Refusal::Denied`].
pub const NOT_ALLOWED: &str = "You are not allowed to make this change.";

const NO_REPLY: &str = "org.freedesktop.DBus.Error.NoReply";

/// The refusal `e` is, when its error name passes `refused` or is `NoReply`, or
/// it is the connection's method timeout; None for any other error. `dismissed`, normally [`dismissed`], is asked only for a
/// refusal, since asking forgets the dismissal.
pub fn refusal(
    e: &zbus::Error,
    refused: impl FnOnce(&str) -> bool,
    dismissed: impl FnOnce() -> bool,
) -> Option<Refusal> {
    if let zbus::Error::InputOutput(io) = e
        && io.kind() == std::io::ErrorKind::TimedOut
    {
        return Some(Refusal::TimedOut);
    }
    let name = crate::error::name(e)?;
    if name == NO_REPLY {
        return Some(Refusal::TimedOut);
    }
    refused(&name).then(|| {
        if dismissed() {
            Refusal::Dismissed
        } else {
            Refusal::Denied
        }
    })
}

/// Whether polkit lets the owner of `conn` do `action` with no password prompt. A service that
/// asks polkit with interaction allowed whatever the call's flags (fwupd does) prompts otherwise.
/// False when polkit does not answer.
pub async fn allowed_without_prompt(conn: &zbus::Connection, action: &str) -> bool {
    check(conn, action).await.is_some_and(|(yes, _)| yes)
}

/// [`allowed_without_prompt`] for good: a password typed for an earlier call, which polkit keeps
/// a few minutes (`auth_admin_keep`), does not count.
pub async fn allowed_without_password(conn: &zbus::Connection, action: &str) -> bool {
    check(conn, action)
        .await
        .is_some_and(|(yes, details)| lasting(yes, &details))
}

fn lasting(authorized: bool, details: &HashMap<String, String>) -> bool {
    authorized && !details.contains_key("polkit.temporary_authorization_id")
}

/// polkit's `CheckAuthorization` for the owner of `conn`, without interaction: whether it is
/// authorized and the details. None when polkit does not answer.
async fn check(conn: &zbus::Connection, action: &str) -> Option<(bool, HashMap<String, String>)> {
    let name = conn.unique_name()?;
    let subject = (
        "system-bus-name",
        HashMap::from([("name", Variant::from(name.as_str()))]),
    );
    let details: HashMap<&str, &str> = HashMap::new();
    let reply = conn
        .call_method(
            Some("org.freedesktop.PolicyKit1"),
            "/org/freedesktop/PolicyKit1/Authority",
            Some("org.freedesktop.PolicyKit1.Authority"),
            "CheckAuthorization",
            &(subject, action, details, 0u32, ""),
        )
        .await
        .and_then(|m| {
            m.body()
                .deserialize::<(bool, bool, HashMap<String, String>)>()
        });
    match reply {
        Ok((authorized, _, details)) => Some((authorized, details)),
        Err(e) => {
            log::info!("polkit does not say whether {action} needs a password: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(name: &str) -> zbus::Error {
        crate::error::reply(name, "Not authorized")
    }

    #[test]
    fn a_kept_password_is_no_lasting_permission() {
        let none = HashMap::new();
        assert!(lasting(true, &none));
        assert!(!lasting(false, &none));
        let kept = HashMap::from([(
            "polkit.temporary_authorization_id".to_string(),
            "tmpauthz7".to_string(),
        )]);
        assert!(!lasting(true, &kept));
    }

    #[test]
    fn each_logind_session_has_its_own_agent_name() {
        assert_eq!(name("3").as_deref(), Some("reprise.polkit.session3"));
        assert_eq!(name("c2").as_deref(), Some("reprise.polkit.sessionc2"));
        for bad in ["", "3.1", "3-1", "3 1", "*"] {
            assert_eq!(name(bad), None, "{bad:?} is no logind session id");
        }
    }

    #[test]
    fn a_program_asks_the_agent_of_the_session_polkit_judges_it_by() {
        let record = "NAME=ann\nDISPLAY=3\nSESSIONS=7 3\n";
        let in_session = "0::/user.slice/user-1000.slice/session-7.scope\n";
        assert_eq!(
            agent_name(in_session, record).as_deref(),
            Some("reprise.polkit.session7"),
            "a program in its own session asks that session's agent"
        );
        let app_scope =
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-reprise-x-1.scope\n";
        assert_eq!(
            agent_name(app_scope, record).as_deref(),
            Some("reprise.polkit.session3"),
            "an app scope is judged by the display session"
        );
        assert_eq!(agent_name(app_scope, "NAME=ann\nSESSIONS=7\n"), None);
    }

    #[test]
    fn only_a_refusal_asks_whether_the_prompt_was_dismissed() {
        let refused = |n: &str| n.ends_with("PermissionDenied");
        let denied = error("org.example.PermissionDenied");
        assert_eq!(refusal(&denied, refused, || true), Some(Refusal::Dismissed));
        assert_eq!(refusal(&denied, refused, || false), Some(Refusal::Denied));
        let other = refusal(&error("org.example.Failed"), refused, || {
            panic!("asking forgets a dismissal another error has no claim to")
        });
        assert_eq!(other, None);
        let local = refusal(
            &zbus::Error::InvalidReply,
            |_| true,
            || panic!("only a method error is a refusal"),
        );
        assert_eq!(local, None);
    }

    #[test]
    fn no_reply_is_a_timed_out_prompt_whatever_the_service_refuses_with() {
        let timed_out = refusal(
            &error("org.freedesktop.DBus.Error.NoReply"),
            |n| n.ends_with("PermissionDenied"),
            || panic!("asking forgets a dismissal a timeout has no claim to"),
        );
        assert_eq!(timed_out, Some(Refusal::TimedOut));
    }

    #[test]
    fn the_callers_own_method_timeout_is_a_timed_out_prompt() {
        let io = zbus::Error::from(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "timed out",
        ));
        let timed_out = refusal(
            &io,
            |_| true,
            || panic!("asking forgets a dismissal a timeout has no claim to"),
        );
        assert_eq!(timed_out, Some(Refusal::TimedOut));
        let reset = zbus::Error::from(std::io::Error::from(std::io::ErrorKind::ConnectionReset));
        assert_eq!(refusal(&reset, |_| true, || false), None);
    }

    #[test]
    fn a_reply_named_in_the_dbus_namespace_is_a_refusal_or_a_timeout_too() {
        let reply = |name| crate::error::reply(name, "Permission denied");
        let denied = reply("org.freedesktop.DBus.Error.AccessDenied");
        let refused = |n: &str| n == "org.freedesktop.DBus.Error.AccessDenied";
        assert_eq!(refusal(&denied, refused, || true), Some(Refusal::Dismissed));
        assert_eq!(refusal(&denied, refused, || false), Some(Refusal::Denied));
        let no_reply = reply("org.freedesktop.DBus.Error.NoReply");
        let timed_out = refusal(
            &no_reply,
            |_| false,
            || panic!("asking forgets a dismissal a timeout has no claim to"),
        );
        assert_eq!(timed_out, Some(Refusal::TimedOut));
    }
}
