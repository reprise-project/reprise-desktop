//! A one-off balloon through `org.freedesktop.Notifications`, for a program with no tray item.

use std::collections::HashMap;
use zbus::zvariant::Value;

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

/// Show `summary` and `body` (plain text, escaped here) as `app` with `icon`, a themed icon's
/// name. Blocks for at most [`crate::METHOD_TIMEOUT`].
pub fn notify(app: &str, icon: &str, summary: &str, body: &str) -> zbus::Result<()> {
    let conn = crate::session()?;
    let hints: HashMap<&str, Value<'_>> = HashMap::new();
    let actions: Vec<&str> = Vec::new();
    conn.call_method(
        Some(NAME),
        PATH,
        Some(NAME),
        "Notify",
        &(
            app,
            0u32,
            icon,
            summary,
            escape(body),
            actions,
            hints,
            -1i32,
        ),
    )?;
    Ok(())
}

/// `s` as a body reads it literally: the spec lets a daemon read a markup subset.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
