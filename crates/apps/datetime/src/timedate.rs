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
pub fn apply(change: &Change) -> Result<System, Failure> {
    zbus::block_on(async {
        let conn = connect(None).await.map_err(text)?;
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
        read_system(&conn).await.map_err(text)
    })
}

/// Why [`apply`] stopped.
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    /// The password prompt was dismissed: nothing to tell.
    Dismissed,
    Text(String),
}

fn text(e: zbus::Error) -> Failure {
    Failure::Text(e.to_string())
}

async fn call<B>(conn: &zbus::Connection, method: &str, body: &B) -> Result<(), Failure>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    conn.call_method(Some(DEST), PATH, Some(IFACE), method, body)
        .await
        .map(|_| ())
        .map_err(|e| describe(method, e, reprisebus::polkit::dismissed))
}

/// timedated answers `AccessDenied` for a dismissed prompt and a refused
/// password alike; `dismissed` tells them apart.
fn describe(method: &str, e: zbus::Error, dismissed: impl FnOnce() -> bool) -> Failure {
    let refused = |n: &str| n == "org.freedesktop.DBus.Error.AccessDenied";
    if reprisebus::polkit::refusal(&e, refused, dismissed)
        == Some(reprisebus::polkit::Refusal::Dismissed)
    {
        log::info!("{method}: the password prompt was dismissed");
        return Failure::Dismissed;
    }
    match &e {
        zbus::Error::MethodError(name, Some(msg), _) => {
            log::error!("{method}: {name}: {msg}");
            Failure::Text(msg.clone())
        }
        _ => {
            log::error!("{method}: {e}");
            text(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(name: &'static str, text: &str) -> zbus::Error {
        let msg = zbus::Message::method_call(PATH, "SetNTP")
            .and_then(|b| b.build(&()))
            .expect("a method call");
        zbus::Error::MethodError(
            name.try_into().expect("an error name"),
            Some(text.into()),
            msg,
        )
    }

    #[test]
    fn access_denied_is_a_dismissal_or_a_refusal() {
        let denied = || {
            error(
                "org.freedesktop.DBus.Error.AccessDenied",
                "Permission denied",
            )
        };
        assert_eq!(describe("SetNTP", denied(), || true), Failure::Dismissed);
        assert_eq!(
            describe("SetNTP", denied(), || false),
            Failure::Text("Permission denied".into())
        );
        let other = describe(
            "SetTime",
            error(
                "org.freedesktop.timedate1.AutomaticTimeSyncEnabled",
                "NTP is on",
            ),
            || panic!("only a refusal asks whether the prompt was dismissed"),
        );
        assert_eq!(other, Failure::Text("NTP is on".into()));
    }
}
