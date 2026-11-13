//! systemd-timedated over the system bus. The setters pass `interactive` and block until polkit is answered, so the
//! host runs [`apply`] off its event loop. Reading also says whether the zone may follow the location: geoclue
//! installed, `[privacy] location` on and polkit setting the zone with no password.

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::time::Duration;

use futures_util::StreamExt;

use crate::sheet::{Blocked, Change, System};

const DEST: &str = "org.freedesktop.timedate1";
const PATH: &str = "/org/freedesktop/timedate1";
const IFACE: &str = "org.freedesktop.timedate1";

const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const TIMEZONE: &str = "Timezone";

const READ_TIMEOUT: Duration = Duration::from_secs(2);

const SET_TIMEZONE: &str = "org.freedesktop.timedate1.set-timezone";
const GEOCLUE: &str = "org.freedesktop.GeoClue2";

async fn connect(timeout: Option<Duration>) -> zbus::Result<zbus::Connection> {
    let b = zbus::connection::Builder::system();
    match timeout {
        Some(t) => b.method_timeout(t).build().await,
        None => b.build().await,
    }
}

async fn property<T>(conn: &zbus::Connection, name: &str) -> zbus::Result<T>
where
    T: TryFrom<zbus::OwnedVariant>,
    T::Error: Into<zbus::Error>,
{
    let reply = conn
        .call_method(Some(DEST), PATH, Some(PROPERTIES), "Get", &(IFACE, name))
        .await?;
    let v: zbus::OwnedVariant = reply.body().deserialize()?;
    T::try_from(v).map_err(Into::into)
}

async fn read_system(conn: &zbus::Connection) -> zbus::Result<System> {
    let (auto_zone, location) = repriseconf::auto_time_zone();
    let geoclue = activatable(conn, GEOCLUE).await;
    let allowed = reprisebus::polkit::allowed_without_password(conn, SET_TIMEZONE).await;
    Ok(System {
        zone: property(conn, TIMEZONE).await?,
        ntp: property(conn, "NTP").await?,
        can_ntp: property(conn, "CanNTP").await?,
        auto_zone,
        auto_blocked: Blocked::first(geoclue, location, allowed),
    })
}

/// The bus starts `name` on demand: its service is installed.
async fn activatable(conn: &zbus::Connection, name: &str) -> bool {
    conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "ListActivatableNames",
        &(),
    )
    .await
    .and_then(|m| m.body().deserialize::<Vec<String>>())
    .is_ok_and(|names| names.iter().any(|n| n == name))
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

/// Send timedated's zone each time its `Timezone` changes, until `zones` is dropped, waking the
/// host after each. The zone sent is read back from timedated, not taken from the signal.
pub fn follow_zone(zones: Sender<String>, waker: repriseapp::Waker) {
    let spawned = std::thread::Builder::new()
        .name("reprise-datetime-zone".into())
        .spawn(move || zbus::block_on(follow(zones, waker)));
    if let Err(e) = spawned {
        log::warn!("could not start the time zone thread: {e}");
    }
}

async fn follow(zones: Sender<String>, waker: repriseapp::Waker) {
    let conn = match connect(Some(READ_TIMEOUT)).await {
        Ok(c) => c,
        Err(e) => {
            log::info!("the time zone is not followed: {e}");
            return;
        }
    };
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .path(PATH)
        .interface(PROPERTIES)
        .member("PropertiesChanged")
        .add_arg(IFACE)
        .build();
    let rules = match rule {
        Ok(r) => vec![r.into()],
        Err(e) => {
            log::warn!("no match rule for {PATH}: {e}");
            return;
        }
    };
    let mut signals = reprisebus::signals(&conn, DEST, rules).await;
    // Read once after subscribing, so a change since the sheet's first read is not lost.
    let mut changed = true;
    loop {
        if changed {
            match property::<String>(&conn, TIMEZONE).await {
                Ok(zone) => {
                    if zones.send(zone).is_err() {
                        return;
                    }
                    waker.wake();
                }
                Err(e) => log::info!("timedated did not say its zone: {e}"),
            }
        }
        changed = match signals.next().await {
            Some(reprisebus::Signal::Message(m)) => names_zone(&m),
            Some(reprisebus::Signal::OwnerChanged) => false,
            None => return,
        };
    }
}

/// Whether a `PropertiesChanged` changes or invalidates `Timezone`.
fn names_zone(m: &zbus::Message) -> bool {
    type Changed = (String, HashMap<String, zbus::OwnedVariant>, Vec<String>);
    m.body()
        .deserialize::<Changed>()
        .is_ok_and(|(_, changed, invalidated)| {
            changed.contains_key(TIMEZONE) || invalidated.iter().any(|p| p == TIMEZONE)
        })
}

/// Carry `change` out, then read the settings back. The automatic zone is
/// written to the config first. Network time goes off before a time is
/// set (timedated refuses one while it is on) and on after everything
/// else; a time is local to the zone just set, so the process rereads the
/// zone before converting it. Blocks until polkit is answered.
pub fn apply(change: &Change) -> Result<System, Failure> {
    if let Some(on) = change.auto_zone {
        repriseconf::save_auto_time_zone(on).map_err(Failure::Text)?;
    }
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
    /// timedated stopped waiting for the password prompt.
    TimedOut,
    /// polkit refused the call; timedated's message.
    Denied(String),
    Text(String),
}

impl Failure {
    /// The message box text; None for a dismissal.
    pub fn message(&self) -> Option<String> {
        match self {
            Failure::Dismissed => None,
            Failure::TimedOut => Some(reprisebus::polkit::TIMED_OUT.into()),
            Failure::Denied(msg) | Failure::Text(msg) => {
                Some(format!("The clock could not be changed: {msg}"))
            }
        }
    }

    /// The call that stopped was never authorized, so it changed nothing.
    pub fn refused(&self) -> bool {
        !matches!(self, Failure::Text(_))
    }
}

fn text(e: zbus::Error) -> Failure {
    Failure::Text(e.to_string())
}

async fn call<B>(conn: &zbus::Connection, method: &str, body: &B) -> Result<(), Failure>
where
    B: serde::Serialize + zbus::DynamicType,
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
    match reprisebus::polkit::refusal(&e, refused, dismissed) {
        Some(reprisebus::polkit::Refusal::Dismissed) => {
            log::info!("{method}: the password prompt was dismissed");
            return Failure::Dismissed;
        }
        Some(reprisebus::polkit::Refusal::TimedOut) => {
            log::warn!("{method}: timedated stopped waiting for the password prompt");
            return Failure::TimedOut;
        }
        Some(reprisebus::polkit::Refusal::Denied) => {
            log::warn!("{method}: {e}");
            return Failure::Denied(message(e));
        }
        None => {}
    }
    log::error!("{method}: {e}");
    Failure::Text(message(e))
}

fn message(e: zbus::Error) -> String {
    match reprisebus::error::message(&e) {
        Some(msg) => msg.into(),
        None => e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisebus::error::reply as error;

    #[test]
    fn access_denied_is_a_dismissal_or_a_refusal() {
        let denied = || {
            error(
                "org.freedesktop.DBus.Error.AccessDenied",
                "Permission denied",
            )
        };
        assert_eq!(describe("SetNTP", denied(), || true), Failure::Dismissed);
        let refused = describe("SetNTP", denied(), || false);
        assert_eq!(refused, Failure::Denied("Permission denied".into()));
        assert!(refused.refused());
        let other = describe(
            "SetTime",
            error(
                "org.freedesktop.timedate1.AutomaticTimeSyncEnabled",
                "NTP is on",
            ),
            || panic!("only a refusal asks whether the prompt was dismissed"),
        );
        assert_eq!(other, Failure::Text("NTP is on".into()));
        assert!(
            !other.refused(),
            "timedated's own error may follow a half-done change"
        );
    }

    #[test]
    fn a_prompt_open_too_long_says_to_try_again() {
        let no_reply = error(
            "org.freedesktop.DBus.Error.NoReply",
            "Method call timed out",
        );
        let failure = describe("SetNTP", no_reply, || {
            panic!("a timeout does not ask whether the prompt was dismissed")
        });
        assert_eq!(failure, Failure::TimedOut);
        assert_eq!(
            failure.message().as_deref(),
            Some("The password prompt was open too long, so nothing was changed. Try again.")
        );
        assert_eq!(Failure::Dismissed.message(), None);
        assert_eq!(
            Failure::Text("NTP is on".into()).message().as_deref(),
            Some("The clock could not be changed: NTP is on")
        );
    }
}
