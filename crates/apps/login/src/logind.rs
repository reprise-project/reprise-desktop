//! logind and localed on the system bus: the proxies, and the reads and calls the daemon makes.

use std::time::Duration;

use futures_util::StreamExt;
use repriselogin::greeter::{Keyboard, PowerAction};
use zbus::OwnedObjectPath;
use zbus::proxy::CacheProperties;
use zbus::{Connection, names::InterfaceName};

use crate::daemon::LogindSession;

const LOGIN1: &str = "org.freedesktop.login1";
const SESSION: &str = "org.freedesktop.login1.Session";
const SEAT0: &str = "/org/freedesktop/login1/seat/seat0";
/// A session logind keeps after its leader is gone, until its last process exits.
const CLOSING: &str = "closing";
/// How long a method call on the system bus waits for its reply.
const METHOD_TIMEOUT: Duration = Duration::from_secs(5);

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
pub trait Manager {
    /// Each session as (id, uid, user name, seat, path).
    fn list_sessions(&self) -> zbus::Result<Vec<(String, u32, String, String, OwnedObjectPath)>>;
    #[zbus(name = "GetSessionByPID")]
    fn get_session_by_pid(&self, pid: u32) -> zbus::Result<OwnedObjectPath>;
    fn get_session(&self, id: &str) -> zbus::Result<OwnedObjectPath>;
    fn power_off(&self, interactive: bool) -> zbus::Result<()>;
    fn reboot(&self, interactive: bool) -> zbus::Result<()>;
    fn terminate_session(&self, id: &str) -> zbus::Result<()>;
    fn unlock_session(&self, id: &str) -> zbus::Result<()>;
    fn lock_session(&self, id: &str) -> zbus::Result<()>;
    #[zbus(signal)]
    fn session_removed(&self, id: String, path: OwnedObjectPath) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Seat",
    default_service = "org.freedesktop.login1"
)]
pub trait Seat {
    fn switch_to(&self, vtnr: u32) -> zbus::Result<()>;
    #[zbus(property)]
    fn active_session(&self) -> zbus::Result<(String, OwnedObjectPath)>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
pub trait Session {
    #[zbus(property)]
    fn id(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn user(&self) -> zbus::Result<(u32, OwnedObjectPath)>;
    #[zbus(property, name = "VTNr")]
    fn vtnr(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn seat(&self) -> zbus::Result<(String, OwnedObjectPath)>;
    #[zbus(property)]
    fn class(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn service(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn leader(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn locked_hint(&self) -> zbus::Result<bool>;
}

#[zbus::proxy(
    interface = "org.freedesktop.locale1",
    default_service = "org.freedesktop.locale1",
    default_path = "/org/freedesktop/locale1"
)]
pub trait Locale {
    #[zbus(property, name = "X11Layout")]
    fn x11_layout(&self) -> zbus::Result<String>;
    #[zbus(property, name = "X11Variant")]
    fn x11_variant(&self) -> zbus::Result<String>;
    #[zbus(property, name = "X11Options")]
    fn x11_options(&self) -> zbus::Result<String>;
}

/// The console keyboard as localed names it; an empty or missing layout is `us`.
pub fn keyboard_from(
    layout: Option<String>,
    variant: Option<String>,
    options: Option<String>,
) -> Keyboard {
    Keyboard {
        layout: layout
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| "us".into()),
        variant: variant.unwrap_or_default(),
        options: options.unwrap_or_default(),
    }
}

/// The daemon's system bus connection with logind's manager on it.
#[derive(Clone)]
pub struct Logind {
    conn: Connection,
    manager: ManagerProxy<'static>,
}

impl Logind {
    pub async fn connect() -> zbus::Result<Logind> {
        let conn = zbus::connection::Builder::system()
            .method_timeout(METHOD_TIMEOUT)
            .build()
            .await?;
        let manager = ManagerProxy::builder(&conn)
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        Ok(Logind { conn, manager })
    }

    pub fn manager(&self) -> &ManagerProxy<'static> {
        &self.manager
    }

    async fn session(&self, path: OwnedObjectPath) -> zbus::Result<SessionProxy<'static>> {
        SessionProxy::builder(&self.conn)
            .path(path)
            .cache_properties(CacheProperties::No)
            .build()
            .await
    }

    /// logind's sessions, leaving out those closing (their user has left them) and those
    /// whose properties can't be read.
    pub async fn sessions(&self) -> zbus::Result<Vec<LogindSession>> {
        let mut found = Vec::new();
        for (id, uid, user, seat, path) in self.manager.list_sessions().await? {
            match self.describe(path).await {
                Ok(Some((vt, class, service))) => found.push(LogindSession {
                    id,
                    user,
                    uid,
                    vt,
                    seat,
                    class,
                    service,
                }),
                Ok(None) => {}
                Err(e) => log::warn!("logind session {id}: {e}"),
            }
        }
        Ok(found)
    }

    /// The VT, class and service of the session at `path`; `None` when it is closing.
    async fn describe(&self, path: OwnedObjectPath) -> zbus::Result<Option<(u16, String, String)>> {
        let s = self.session(path).await?;
        if s.state().await? == CLOSING {
            return Ok(None);
        }
        let vt = u16::try_from(s.vtnr().await?).unwrap_or(0);
        Ok(Some((vt, s.class().await?, s.service().await?)))
    }

    /// The pid of session `id`'s leader.
    pub async fn leader(&self, id: &str) -> Option<u32> {
        let path = self.manager.get_session(id).await.ok()?;
        self.session(path).await.ok()?.leader().await.ok()
    }

    /// The open logind session of process `pid`.
    pub async fn session_of(&self, pid: u32) -> Option<String> {
        let path = self.manager.get_session_by_pid(pid).await.ok()?;
        let s = self.session(path).await.ok()?;
        match s.state().await.ok()?.as_str() {
            CLOSING => None,
            _ => s.id().await.ok(),
        }
    }

    /// Whether session `id` is gone or closing.
    pub async fn ended(&self, id: &str) -> bool {
        let Ok(path) = self.manager.get_session(id).await else {
            return true;
        };
        match self.session(path).await {
            Ok(s) => s.state().await.is_ok_and(|state| state == CLOSING),
            Err(_) => true,
        }
    }

    async fn seat0(&self) -> zbus::Result<SeatProxy<'static>> {
        SeatProxy::builder(&self.conn)
            .path(SEAT0)
            .cache_properties(CacheProperties::No)
            .build()
            .await
    }

    /// seat0's active session, `None` when it has none.
    pub async fn seat_active(&self) -> Option<String> {
        let (id, _) = self.seat0().await.ok()?.active_session().await.ok()?;
        (!id.is_empty()).then_some(id)
    }

    pub async fn switch_to(&self, vt: u16) -> zbus::Result<()> {
        self.seat0().await?.switch_to(vt.into()).await
    }

    pub async fn power(&self, action: PowerAction) -> zbus::Result<()> {
        match action {
            PowerAction::PowerOff => self.manager.power_off(false).await,
            PowerAction::Reboot => self.manager.reboot(false).await,
        }
    }

    /// `UnlockSession(id)`, then true once the session's `LockedHint` is false, or false
    /// when it is still set after `wait`.
    pub async fn unlock(&self, id: &str, wait: Duration) -> zbus::Result<bool> {
        let path = self.manager.get_session(id).await?;
        let props = zbus::fdo::PropertiesProxy::builder(&self.conn)
            .destination(LOGIN1)
            .path(path.clone())
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        // Subscribed before the call, so a change in between is seen.
        let mut changes = props.receive_properties_changed().await?;
        self.manager.unlock_session(id).await?;
        let session = self.session(path).await?;
        if !session.locked_hint().await? {
            return Ok(true);
        }
        let iface = InterfaceName::from_static_str_unchecked(SESSION);
        let dropped = async {
            while let Some(change) = changes.next().await {
                let Ok(args) = change.args() else { continue };
                if *args.interface_name() != iface {
                    continue;
                }
                let value = match args.changed_properties().get("LockedHint") {
                    Some(v) => bool::try_from(v).ok(),
                    None if args.invalidated_properties().contains(&"LockedHint") => {
                        session.locked_hint().await.ok()
                    }
                    None => continue,
                };
                if value == Some(false) {
                    return true;
                }
            }
            false
        };
        Ok(tokio::time::timeout(wait, dropped).await.unwrap_or(false))
    }

    /// The console keyboard from localed, `us` when it names none.
    pub async fn keyboard(&self) -> Keyboard {
        let locale = LocaleProxy::builder(&self.conn)
            .cache_properties(CacheProperties::No)
            .build()
            .await;
        let Ok(locale) = locale else {
            return keyboard_from(None, None, None);
        };
        keyboard_from(
            locale.x11_layout().await.ok(),
            locale.x11_variant().await.ok(),
            locale.x11_options().await.ok(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_falls_back_to_us() {
        let us = Keyboard {
            layout: "us".into(),
            ..Keyboard::default()
        };
        assert_eq!(keyboard_from(None, None, None), us);
        assert_eq!(keyboard_from(Some(String::new()), None, None), us);
        assert_eq!(
            keyboard_from(
                Some("de,us".into()),
                Some("nodeadkeys,".into()),
                Some("grp:alt_shift_toggle".into())
            ),
            Keyboard {
                layout: "de,us".into(),
                variant: "nodeadkeys,".into(),
                options: "grp:alt_shift_toggle".into(),
            }
        );
    }
}
