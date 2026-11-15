//! Files picked through the desktop portal's FileChooser, which shows the session's own Open
//! dialog; the call blocks its thread until the dialog closes.

use reprisebus::portal;
use std::collections::HashMap;
use zbus::{Connection, OwnedVariant, Variant};

const CHOOSER: &str = "org.freedesktop.portal.FileChooser";

/// The file URIs of an accepted Response's results.
fn uris(results: &HashMap<String, OwnedVariant>) -> Vec<String> {
    (results.get("uris").cloned())
        .and_then(|v| Vec::<String>::try_from(v).ok())
        .unwrap_or_default()
}

/// The files picked in an Open dialog titled `title` that takes several, as file URIs; none when
/// it was cancelled.
pub async fn files(conn: &Connection, title: &str, accept: &str) -> zbus::Result<Vec<String>> {
    let picked = portal::request(conn, CHOOSER, "OpenFile", None, |mut options| {
        options.insert("multiple", Variant::from(true));
        options.insert("accept_label", Variant::from(accept.to_string()));
        ("", title, options)
    })
    .await;
    match picked {
        Ok(results) => Ok(uris(&results)),
        Err(portal::Error::Bus(e)) => Err(e),
        Err(e) => {
            log::debug!("no files picked: {e}");
            Ok(Vec::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_files_are_the_results_uris() {
        let list = vec!["file:///home/u/a.jpg".to_string()];
        let results = HashMap::from([(
            "uris".to_string(),
            OwnedVariant::try_from(Variant::from(list.clone())).unwrap(),
        )]);
        assert_eq!(uris(&results), list);
        assert!(uris(&HashMap::new()).is_empty());
    }
}
