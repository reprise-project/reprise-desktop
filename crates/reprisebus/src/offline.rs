//! PackageKit's offline updates: whether an update is prepared and triggered. systemd installs a
//! triggered update at boot (`/system-update`).

use crate::{Props, prop};

use zbus::Connection;
use zbus::proxy::{CacheProperties, Proxy};

pub const NAME: &str = "org.freedesktop.PackageKit";
pub const PATH: &str = "/org/freedesktop/PackageKit";
pub const OFFLINE: &str = "org.freedesktop.PackageKit.Offline";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub prepared: bool,
    pub triggered: bool,
}

impl Status {
    fn from_props(props: &Props) -> Status {
        Status {
            prepared: prop(props, "UpdatePrepared").unwrap_or(false),
            triggered: prop(props, "UpdateTriggered").unwrap_or(false),
        }
    }
}

/// The offline update's state, None when PackageKit is absent or does not answer. Reading starts
/// PackageKit when it is not running.
pub async fn status(conn: &Connection) -> Option<Status> {
    let props = async {
        let proxy: Proxy<'_> = zbus::proxy::Builder::new(conn)
            .destination(NAME)
            .path(PATH)
            .interface(PROPERTIES)
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        proxy.call::<_, _, Props>("GetAll", &(OFFLINE,)).await
    };
    match props.await {
        Ok(p) => Some(Status::from_props(&p)),
        Err(e) => {
            log::info!("offline update: PackageKit unavailable ({e})");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testbus::{bus, bus_with_no_services, connect, serve};

    struct Fake {
        prepared: bool,
        triggered: bool,
    }

    #[zbus::interface(name = "org.freedesktop.PackageKit.Offline")]
    impl Fake {
        #[zbus(property)]
        fn update_prepared(&self) -> bool {
            self.prepared
        }
        #[zbus(property)]
        fn update_triggered(&self) -> bool {
            self.triggered
        }
    }

    #[test]
    fn a_prepared_and_triggered_update_is_read() {
        let Some(bus) = bus() else {
            return;
        };
        zbus::block_on(async move {
            let pk = connect(&bus.1).await;
            let fake = Fake {
                prepared: true,
                triggered: true,
            };
            serve(&pk, PATH, fake).await;
            pk.request_name(NAME).await.unwrap();
            let conn = connect(&bus.1).await;
            let both = Status {
                prepared: true,
                triggered: true,
            };
            assert_eq!(status(&conn).await, Some(both));
        });
    }

    #[test]
    fn no_packagekit_means_nothing_known() {
        let Some(bus) = bus_with_no_services() else {
            return;
        };
        let conn = zbus::block_on(connect(&bus.1));
        assert_eq!(zbus::block_on(status(&conn)), None);
    }
}
