//! colord's profiles on the system bus, read only: the files of every profile it knows, imported
//! and assigned to devices ones included.

use std::path::PathBuf;

use zbus::proxy::CacheProperties;
use zbus::{Connection, OwnedObjectPath, Proxy};

pub const NAME: &str = "org.freedesktop.ColorManager";
const PATH: &str = "/org/freedesktop/ColorManager";
const PROFILE: &str = "org.freedesktop.ColorManager.Profile";
/// More profiles than a system has; the rest are left out.
const MAX_PROFILES: usize = 512;

async fn files(conn: &Connection) -> zbus::Result<Vec<PathBuf>> {
    let reply = conn
        .call_method(Some(NAME), PATH, Some(NAME), "GetProfiles", &())
        .await?;
    let paths: Vec<OwnedObjectPath> = reply.body().deserialize()?;
    let mut out = Vec::new();
    for path in paths.into_iter().take(MAX_PROFILES) {
        let p = zbus::proxy::Builder::<Proxy>::new(conn)
            .destination(NAME)
            .path(path.as_str())
            .interface(PROFILE)
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        if let Ok(f) = p.get_property::<String>("Filename").await
            && !f.is_empty()
        {
            out.push(PathBuf::from(f));
        }
    }
    Ok(out)
}

/// The profile files colord lists while it runs; empty when it doesn't, since this never starts it.
pub fn profile_files() -> Vec<PathBuf> {
    let Ok(conn) = crate::system() else {
        return Vec::new();
    };
    crate::wait(async {
        crate::name_owner(&conn, NAME).await?;
        files(&conn)
            .await
            .inspect_err(|e| log::info!("colord: {e}"))
            .ok()
    })
    .unwrap_or_default()
}
