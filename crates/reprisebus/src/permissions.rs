//! The portals' permission store on the session bus: the answers the portals keep per app,
//! read and revoked by Reprise Settings and written by reprise-portal's remembered prompts.

use std::collections::HashMap;

use zbus::OwnedVariant;

const NAME: &str = "org.freedesktop.impl.portal.PermissionStore";
const PATH: &str = "/org/freedesktop/impl/portal/PermissionStore";
const INTERFACE: &str = "org.freedesktop.impl.portal.PermissionStore";
/// What the store answers for a table or id it has never written.
const NOT_FOUND: &str = "org.freedesktop.portal.Error.NotFound";

/// The programs `table` holds an answer for under `id`, by app id, each with its stored
/// values. Sorted by app id; empty when the store has never written the table or id.
pub fn answers(table: &str, id: &str) -> zbus::Result<Vec<(String, Vec<String>)>> {
    let conn = crate::session()?;
    let reply =
        crate::wait(conn.call_method(Some(NAME), PATH, Some(INTERFACE), "Lookup", &(table, id)));
    let reply = match reply {
        Err(e) if crate::error::name(&e).is_some_and(|n| n == NOT_FOUND) => {
            return Ok(Vec::new());
        }
        r => r?,
    };
    let (perms, _): (HashMap<String, Vec<String>>, OwnedVariant) = reply.body().deserialize()?;
    let mut out: Vec<_> = perms.into_iter().collect();
    out.sort();
    Ok(out)
}

/// An entry of a table: its id, each program it holds a permission for with the values, and
/// the entry's data.
pub struct Entry {
    pub id: String,
    pub apps: HashMap<String, Vec<String>>,
    pub data: OwnedVariant,
}

/// Every entry of `table` in the store on `conn`; empty when the store has never written the
/// table. An entry deleted between the listing and its lookup is left out.
pub async fn entries(conn: &zbus::Connection, table: &str) -> zbus::Result<Vec<Entry>> {
    let listed = conn
        .call_method(Some(NAME), PATH, Some(INTERFACE), "List", &(table,))
        .await;
    let ids: Vec<String> = match listed {
        Err(e) if crate::error::name(&e).is_some_and(|n| n == NOT_FOUND) => {
            return Ok(Vec::new());
        }
        r => r?.body().deserialize()?,
    };
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let found = conn
            .call_method(Some(NAME), PATH, Some(INTERFACE), "Lookup", &(table, &id))
            .await;
        let Ok(found) = found else { continue };
        if let Ok((apps, data)) = found.body().deserialize() {
            out.push(Entry { id, apps, data });
        }
    }
    Ok(out)
}

/// Drop `app`'s answer under `table` and `id`: the portal asks it again next time.
pub fn forget(table: &str, id: &str, app: &str) -> zbus::Result<()> {
    crate::wait(crate::session()?.call_method(
        Some(NAME),
        PATH,
        Some(INTERFACE),
        "DeletePermission",
        &(table, id, app),
    ))?;
    Ok(())
}

/// Drop `app` from restore token `id` of `table`, and the token with it when no other program
/// holds it.
pub fn forget_token(table: &str, id: &str, app: &str) -> zbus::Result<()> {
    if answers(table, id)?.iter().any(|(a, _)| a != app) {
        return forget(table, id, app);
    }
    let conn = crate::session()?;
    let deleted =
        crate::wait(conn.call_method(Some(NAME), PATH, Some(INTERFACE), "Delete", &(table, id)));
    match deleted {
        Err(e) if crate::error::name(&e).is_some_and(|n| n == NOT_FOUND) => Ok(()),
        r => r.map(drop),
    }
}

/// `app_id`'s permission in entry `id` of `table`; None when the store holds none.
pub fn app_permission(table: &str, id: &str, app_id: &str) -> zbus::Result<Option<Vec<String>>> {
    let conn = crate::session()?;
    let reply =
        crate::wait(conn.call_method(Some(NAME), PATH, Some(INTERFACE), "Lookup", &(table, id)));
    let reply = match reply {
        Err(e) if crate::error::name(&e).is_some_and(|n| n == NOT_FOUND) => return Ok(None),
        r => r?,
    };
    let (mut perms, _): (HashMap<String, Vec<String>>, OwnedVariant) =
        reply.body().deserialize()?;
    Ok(perms.remove(app_id))
}

/// Store `perms` as `app_id`'s permission in entry `id` of `table`, creating both if missing.
pub fn set_app_permission(table: &str, id: &str, app_id: &str, perms: &[&str]) -> zbus::Result<()> {
    crate::wait(crate::session()?.call_method(
        Some(NAME),
        PATH,
        Some(INTERFACE),
        "SetPermission",
        &(table, true, id, app_id, perms),
    ))?;
    Ok(())
}
