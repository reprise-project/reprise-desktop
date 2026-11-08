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

/// Whether reprise-login is on this computer and running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Manager {
    Missing,
    Stopped,
    Running,
}

/// [`Manager`] for [`UNIT`] through the program's [`crate::system`] connection, blocking; a bus
/// that fails reads as stopped.
pub fn manager() -> Manager {
    let read = async {
        let system = crate::system()?;
        let path: zbus::OwnedObjectPath = system
            .call_method(
                Some(SYSTEMD),
                "/org/freedesktop/systemd1",
                Some("org.freedesktop.systemd1.Manager"),
                "LoadUnit",
                &(UNIT,),
            )
            .await?
            .body()
            .deserialize()?;
        let unit: zbus::Proxy = zbus::proxy::Builder::new(&system)
            .destination(SYSTEMD)
            .path(&path)
            .interface("org.freedesktop.systemd1.Unit")
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await?;
        let load: String = unit.get_property("LoadState").await?;
        let active: String = unit.get_property("ActiveState").await?;
        zbus::Result::Ok(manager_of(&load, &active))
    };
    crate::wait(read).unwrap_or_else(|e| {
        log::debug!("{UNIT}: {e}");
        Manager::Stopped
    })
}

fn manager_of(load: &str, active: &str) -> Manager {
    match (load, counts(active)) {
        ("not-found", _) => Manager::Missing,
        (_, true) => Manager::Running,
        (_, false) => Manager::Stopped,
    }
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

    #[test]
    fn a_unit_not_found_is_missing_and_a_loaded_one_runs_while_active() {
        assert_eq!(manager_of("not-found", "inactive"), Manager::Missing);
        assert_eq!(manager_of("loaded", "active"), Manager::Running);
        assert_eq!(manager_of("loaded", "inactive"), Manager::Stopped);
        assert_eq!(manager_of("masked", "inactive"), Manager::Stopped);
    }
}
