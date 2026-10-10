//! systemd-timedated over the system bus. The setters pass `interactive` and block until polkit is answered, so the
//! host runs [`apply`] off its event loop.

use std::time::Duration;

use crate::sheet::{Change, System};

const DEST: &str = "org.freedesktop.timedate1";
const PATH: &str = "/org/freedesktop/timedate1";
const IFACE: &str = "org.freedesktop.timedate1";

const READ_TIMEOUT: Duration = Duration::from_secs(2);

async fn connect(timeout: Option<Duration>) -> zbus::Result<zbus::Connection> {
    let b = zbus::connection::Builder::system()?;
    match timeout {
        Some(t) => b.method_timeout(t).build().await,
        None => b.build().await,
    }
}

async fn property<T>(conn: &zbus::Connection, name: &str) -> zbus::Result<T>
where
    T: TryFrom<zbus::zvariant::OwnedValue>,
    T::Error: Into<zbus::Error>,
{
    let reply = conn
        .call_method(
            Some(DEST),
            PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(IFACE, name),
        )
        .await?;
    let v: zbus::zvariant::OwnedValue = reply.body().deserialize()?;
    T::try_from(v).map_err(Into::into)
}

async fn read_system(conn: &zbus::Connection) -> zbus::Result<System> {
    Ok(System {
        zone: property(conn, "Timezone").await?,
        ntp: property(conn, "NTP").await?,
        can_ntp: property(conn, "CanNTP").await?,
    })
}

/// timedated's settings and its list of zones. The list is empty when
/// the call is missing (an old systemd); the sheet then offers only the
/// current zone.
pub fn read() -> zbus::Result<(System, Vec<String>)> {
    zbus::block_on(async {
        let conn = connect(Some(READ_TIMEOUT)).await?;
        let system = read_system(&conn).await?;
        let zones = match conn
            .call_method(Some(DEST), PATH, Some(IFACE), "ListTimezones", &())
            .await
        {
            Ok(reply) => reply.body().deserialize()?,
            Err(e) => {
                log::warn!("timedated lists no zones: {e}");
                Vec::new()
            }
        };
        Ok((system, zones))
    })
}

/// Carry `change` out, then read the settings back. Network time goes off
/// before a time is set (timedated refuses one while it is on) and on
/// after everything else; a time is local to the zone just set, so the
/// process rereads the zone before converting it. Blocks until polkit is
/// answered.
pub fn apply(change: &Change) -> Result<System, String> {
    zbus::block_on(async {
        let conn = connect(None).await.map_err(|e| e.to_string())?;
        if change.ntp == Some(false) {
            call(&conn, "SetNTP", &(false, true)).await?;
        }
        if let Some(zone) = &change.zone {
            call(&conn, "SetTimezone", &(zone.as_str(), true)).await?;
            reprisefs::locale::refresh_zone();
        }
        if let Some(t) = change.time {
            let secs = reprisefs::locale::mktime(
                t.year,
                t.month as i32,
                t.day as i32,
                t.h as i32,
                t.m as i32,
                t.s as i32,
            );
            call(&conn, "SetTime", &(secs * 1_000_000, false, true)).await?;
        }
        if change.ntp == Some(true) {
            call(&conn, "SetNTP", &(true, true)).await?;
        }
        read_system(&conn).await.map_err(|e| e.to_string())
    })
}

async fn call<B>(conn: &zbus::Connection, method: &str, body: &B) -> Result<(), String>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    conn.call_method(Some(DEST), PATH, Some(IFACE), method, body)
        .await
        .map(|_| ())
        .map_err(|e| describe(method, e))
}

fn describe(method: &str, e: zbus::Error) -> String {
    match &e {
        zbus::Error::MethodError(name, Some(msg), _) => {
            log::error!("{method}: {name}: {msg}");
            msg.clone()
        }
        _ => {
            log::error!("{method}: {e}");
            e.to_string()
        }
    }
}
