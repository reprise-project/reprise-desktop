//! Files picked through the desktop portal's FileChooser, which shows the session's own Open
//! dialog; the call blocks its thread until the dialog closes.

use futures_util::stream::StreamExt;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use zbus::{Connection, MessageStream, OwnedVariant, Variant};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const CHOOSER: &str = "org.freedesktop.portal.FileChooser";
const REQUEST: &str = "org.freedesktop.portal.Request";

/// The request object the portal answers on, which the spec derives from our unique name and
/// the handle token, so the Response can be subscribed before the call.
fn request_path(unique: &str, token: &str) -> String {
    let sender = unique.trim_start_matches(':').replace('.', "_");
    format!("{PORTAL_PATH}/request/{sender}/{token}")
}

/// The file URIs of a Response's results; none for a cancel (any code but 0).
fn uris(code: u32, results: &HashMap<String, OwnedVariant>) -> Vec<String> {
    if code != 0 {
        return Vec::new();
    }
    (results.get("uris").cloned())
        .and_then(|v| Vec::<String>::try_from(v).ok())
        .unwrap_or_default()
}

/// The files picked in an Open dialog titled `title` that takes several, as file URIs; none when
/// it was cancelled.
pub async fn files(conn: &Connection, title: &str, accept: &str) -> zbus::Result<Vec<String>> {
    static SERIAL: AtomicU32 = AtomicU32::new(0);
    let token = format!("reprise_phone_{}", SERIAL.fetch_add(1, Ordering::Relaxed));
    let unique = conn
        .unique_name()
        .map(|n| n.to_string())
        .unwrap_or_default();
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(REQUEST)
        .member("Response")
        .path(request_path(&unique, &token))
        .build()?;
    let mut responses = MessageStream::for_match_rule(rule, conn, None).await?;
    let options: HashMap<&str, Variant<'_>> = HashMap::from([
        ("handle_token", Variant::from(token.as_str())),
        ("multiple", Variant::from(true)),
        ("accept_label", Variant::from(accept)),
    ]);
    conn.call_method(
        Some(PORTAL),
        PORTAL_PATH,
        Some(CHOOSER),
        "OpenFile",
        &("", title, options),
    )
    .await?;
    let Some(m) = responses.next().await else {
        return Ok(Vec::new());
    };
    let (code, results): (u32, HashMap<String, OwnedVariant>) = m?.body().deserialize()?;
    Ok(uris(code, &results))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_path_follows_the_unique_name_and_token() {
        assert_eq!(
            request_path(":1.42", "reprise_phone_0"),
            "/org/freedesktop/portal/desktop/request/1_42/reprise_phone_0"
        );
    }

    #[test]
    fn only_an_accepted_response_carries_files() {
        let list = vec!["file:///home/u/a.jpg".to_string()];
        let results = HashMap::from([(
            "uris".to_string(),
            OwnedVariant::try_from(Variant::from(list.clone())).unwrap(),
        )]);
        assert_eq!(uris(0, &results), list);
        assert!(uris(1, &results).is_empty(), "cancelled");
        assert!(uris(0, &HashMap::new()).is_empty());
    }
}
