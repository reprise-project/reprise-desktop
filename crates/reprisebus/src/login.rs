//! Whether reprise-login runs this machine's logins.

const UNIT: &str = "reprise-login.service";
const SYSTEMD: &str = "org.freedesktop.systemd1";

/// Whether the system manager has [`UNIT`] active; false when it is not loaded or the bus fails.
pub async fn manager_active(system: &zbus::Connection) -> bool {
    match active_state(system).await {
        Ok(state) => counts(&state),
        Err(e) => {
            log::debug!("{UNIT}: {e}");
            false
        }
    }
}

/// [`manager_active`] on the program's [`crate::system`] connection, blocking.
pub fn manager_runs() -> bool {
    match crate::system() {
        Ok(system) => crate::wait(manager_active(&system)),
        Err(e) => {
            log::debug!("no system bus: {e}");
            false
        }
    }
}

async fn active_state(system: &zbus::Connection) -> zbus::Result<String> {
    let path: zbus::OwnedObjectPath = system
        .call_method(
            Some(SYSTEMD),
            "/org/freedesktop/systemd1",
            Some("org.freedesktop.systemd1.Manager"),
            "GetUnit",
            &(UNIT,),
        )
        .await?
        .body()
        .deserialize()?;
    let unit: zbus::Proxy = zbus::proxy::Builder::new(system)
        .destination(SYSTEMD)
        .path(&path)
        .interface("org.freedesktop.systemd1.Unit")
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await?;
    unit.get_property("ActiveState").await
}

fn counts(state: &str) -> bool {
    state == "active"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_active_unit_counts() {
        assert!(counts("active"));
        for state in ["activating", "inactive", ""] {
            assert!(!counts(state), "{state:?}");
        }
    }
}
