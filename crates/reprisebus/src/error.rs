//! The name and message of a D-Bus error reply. zbus reports a reply whose name `fdo::Error`
//! defines (`org.freedesktop.DBus.Error.*`) as `Error::FDO`, any other as `Error::MethodError`.

use zbus::DBusError;
use zbus::names::ErrorName;

/// The error name of `e` when it is an error reply; None for any other error.
pub fn name(e: &zbus::Error) -> Option<ErrorName<'_>> {
    match e {
        zbus::Error::MethodError(name, _, _) => Some(name.as_ref()),
        zbus::Error::FDO(e) => Some(e.name()),
        zbus::Error::DBus(e) => Some(e.name()),
        _ => None,
    }
}

/// The message an error reply carries; None for an empty one or any other error.
pub fn message(e: &zbus::Error) -> Option<&str> {
    match e {
        zbus::Error::MethodError(_, detail, _) => detail.as_deref(),
        zbus::Error::FDO(e) => e.description(),
        zbus::Error::DBus(e) => e.description(),
        _ => None,
    }
    .filter(|m| !m.is_empty())
}

/// What to tell the user when a call that may open a password prompt fails with `e`; None when
/// `dismissed` (usually [`crate::polkit::dismissed`]) says the prompt was dismissed. A refusal
/// and a failure of the call itself read alike, so the prompt is asked about first.
pub fn to_tell(e: zbus::Error, dismissed: impl FnOnce() -> bool) -> Option<String> {
    if name(&e).is_some() && dismissed() {
        return None;
    }
    Some(message(&e).map_or_else(|| e.to_string(), String::from))
}

/// The error a call fails with when the peer answers an error reply named `name` carrying
/// `message`, as tests build one. Panics on an invalid error name.
pub fn reply(name: &str, message: &str) -> zbus::Error {
    let call = zbus::Message::method_call("/", "M")
        .build(&())
        .expect("a method call");
    let reply = zbus::Message::error(&call.header(), name)
        .build(&(message,))
        .expect("an error reply");
    zbus::Error::from(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_keeps_its_name_and_message_in_and_out_of_the_dbus_namespace() {
        for n in [
            "org.freedesktop.DBus.Error.AccessDenied",
            "org.freedesktop.DBus.Error.NoReply",
            "org.freedesktop.DBus.Error.UnknownMethod",
            "org.freedesktop.login1.NoSuchSession",
        ] {
            let e = reply(n, "Permission denied");
            assert_eq!(
                name(&e).as_deref(),
                Some(n),
                "{n} is read back from its reply"
            );
            assert_eq!(
                message(&e),
                Some("Permission denied"),
                "{n} keeps its message"
            );
        }
        assert_eq!(
            message(&reply("org.freedesktop.DBus.Error.Failed", "")),
            None
        );
        assert_eq!(name(&zbus::Error::InvalidReply), None);
    }

    #[test]
    fn an_error_reply_asks_about_the_prompt_and_tells_its_message() {
        let unknown = || {
            reply(
                "org.freedesktop.DBus.Error.UnknownMethod",
                "No such method \u{201c}EnableChargeThreshold\u{201d}",
            )
        };
        assert_eq!(to_tell(unknown(), || true), None);
        assert_eq!(
            to_tell(unknown(), || false).as_deref(),
            Some("No such method \u{201c}EnableChargeThreshold\u{201d}")
        );
        let lost = zbus::Error::from(std::io::Error::from(std::io::ErrorKind::ConnectionReset));
        let told = to_tell(lost, || panic!("only an error reply follows a prompt"));
        assert!(told.is_some());
    }
}
