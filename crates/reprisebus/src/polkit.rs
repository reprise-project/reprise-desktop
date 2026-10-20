//! reprise-polkit's record of dismissed password prompts, on the session bus.
//! A service behind polkit answers the same error for a dismissed prompt and
//! a refused password; a program asks here which one it was.
//! [`allowed_without_prompt`] asks polkit itself whether an action needs a password.

use std::collections::HashMap;

use zbus::zvariant::Value;

pub const NAME: &str = "reprise.polkit";
pub const PATH: &str = "/reprise/polkit/Dismissals";
pub const INTERFACE: &str = "reprise.polkit.Dismissals";

/// Whether the last password prompt polkit opened for this process was
/// dismissed, forgetting it once asked. False when reprise-polkit does not
/// answer. Blocks for at most [`crate::METHOD_TIMEOUT`].
pub fn dismissed() -> bool {
    let answer = crate::session().and_then(|conn| {
        conn.call_method(Some(NAME), PATH, Some(INTERFACE), "Take", &())?
            .body()
            .deserialize::<bool>()
    });
    answer.unwrap_or_else(|e| {
        log::warn!("reprise-polkit does not say whether the prompt was dismissed: {e}");
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

const NO_REPLY: &str = "org.freedesktop.DBus.Error.NoReply";

/// The refusal `e` is, when its error name passes `refused` or is `NoReply`, or
/// it is the connection's method timeout; None for any other error. `dismissed`, normally [`dismissed`], is asked only for a
/// refusal, since asking forgets the dismissal.
pub fn refusal(
    e: &zbus::Error,
    refused: impl FnOnce(&str) -> bool,
    dismissed: impl FnOnce() -> bool,
) -> Option<Refusal> {
    match e {
        zbus::Error::MethodError(name, _, _) if name.as_str() == NO_REPLY => {
            Some(Refusal::TimedOut)
        }
        zbus::Error::InputOutput(io) if io.kind() == std::io::ErrorKind::TimedOut => {
            Some(Refusal::TimedOut)
        }
        zbus::Error::MethodError(name, _, _) if refused(name.as_str()) => Some(if dismissed() {
            Refusal::Dismissed
        } else {
            Refusal::Denied
        }),
        _ => None,
    }
}

/// Whether polkit lets the owner of `conn` do `action` with no password prompt. A service that
/// asks polkit with interaction allowed whatever the call's flags (fwupd does) prompts otherwise.
/// False when polkit does not answer.
pub async fn allowed_without_prompt(conn: &zbus::Connection, action: &str) -> bool {
    let Some(name) = conn.unique_name() else {
        return false;
    };
    let subject = (
        "system-bus-name",
        HashMap::from([("name", Value::from(name.as_str()))]),
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
        Ok((authorized, _, _)) => authorized,
        Err(e) => {
            log::info!("polkit does not say whether {action} needs a password: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(name: &'static str) -> zbus::Error {
        let msg = zbus::Message::method_call("/", "M")
            .and_then(|b| b.build(&()))
            .expect("a method call");
        zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from(name).expect("an error name"),
            Some("Not authorized".into()),
            msg,
        )
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
}
