//! The portals' permission store on the session bus: the answers the Location
//! portal keeps for Flatpak programs, read and revoked by Reprise Settings.

use std::collections::HashMap;

use zbus::OwnedVariant;

const NAME: &str = "org.freedesktop.impl.portal.PermissionStore";
const PATH: &str = "/org/freedesktop/impl/portal/PermissionStore";
const INTERFACE: &str = "org.freedesktop.impl.portal.PermissionStore";
/// The Location portal's table, and the one id in it.
const LOCATION: &str = "location";
/// What the store answers for a table or id it has never written.
const NOT_FOUND: &str = "org.freedesktop.portal.Error.NotFound";

/// The Flatpak programs the Location portal has an answer for, by app id,
/// and whether each may read the location. Empty when it has none.
pub fn location_answers() -> zbus::Result<Vec<(String, bool)>> {
    let conn = crate::session()?;
    let reply = crate::wait(conn.call_method(
        Some(NAME),
        PATH,
        Some(INTERFACE),
        "Lookup",
        &(LOCATION, LOCATION),
    ));
    let reply = match reply {
        Err(zbus::Error::MethodError(name, _, _)) if name.as_str() == NOT_FOUND => {
            return Ok(Vec::new());
        }
        r => r?,
    };
    let (perms, _): (HashMap<String, Vec<String>>, OwnedVariant) = reply.body().deserialize()?;
    Ok(answers(perms))
}

/// Drop `app_id`'s answer: the portal asks it again next time.
pub fn forget_location(app_id: &str) -> zbus::Result<()> {
    crate::wait(crate::session()?.call_method(
        Some(NAME),
        PATH,
        Some(INTERFACE),
        "DeletePermission",
        &(LOCATION, LOCATION, app_id),
    ))?;
    Ok(())
}

/// Each app's permission is `[accuracy, last use]`; any accuracy but `NONE`
/// allows. Sorted by app id.
fn answers(perms: HashMap<String, Vec<String>>) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = perms
        .into_iter()
        .map(|(app, p)| {
            let allow = p.first().is_some_and(|a| a != "NONE");
            (app, allow)
        })
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_accuracy_but_none_allows_and_apps_sort_by_id() {
        let perms = HashMap::from([
            (
                "org.gnome.Weather".to_string(),
                vec!["NONE".into(), "0".into()],
            ),
            (
                "org.gnome.Maps".to_string(),
                vec!["EXACT".into(), "1234".into()],
            ),
            (
                "org.example.City".to_string(),
                vec!["CITY".into(), "1".into()],
            ),
            ("org.example.Empty".to_string(), Vec::new()),
        ]);
        assert_eq!(
            answers(perms),
            [
                ("org.example.City".to_string(), true),
                ("org.example.Empty".to_string(), false),
                ("org.gnome.Maps".to_string(), true),
                ("org.gnome.Weather".to_string(), false),
            ]
        );
    }
}
