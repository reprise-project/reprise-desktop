//! Keys reported to the AT-SPI registry for a screen reader, the wait for the accessibility bus
//! before a host's first AccessKit adapter ([`when_bus_ready`]), and the watch for its restart
//! ([`on_bus_restart`]).
//!
//! A screen reader under Wayland cannot see the keyboard: Orca learns of keys from the toolkit of
//! whatever has the keyboard, which reports each one to the registry
//! (`DeviceEventController.NotifyListenersSync` on the accessibility bus). It needs them to know that a
//! caret or highlight moved because of an arrow key, and so to say the new line or row.

use std::time::Duration;

use zeroize::Zeroizing;

/// Runs `ready` on a thread of its own once the accessibility bus answers ([`status`]), retrying
/// with [`retry_after`]; never runs it if the bus stays silent. accesskit_unix connects once, when
/// a process makes its first adapter: a call failing while the bus is still starting (a session
/// restart racing the old session's stop job) leaves the process unseen by readers for good, so a
/// host makes its first adapter only after `ready`.
pub fn when_bus_ready(ready: impl FnOnce() + Send + 'static) {
    let run = move || {
        let mut attempt = 0;
        loop {
            match status() {
                Ok(()) => {
                    if attempt > 0 {
                        log::info!("accessibility bus answered after {attempt} retries");
                    }
                    ready();
                    return;
                }
                Err(e) => match retry_after(attempt) {
                    Some(wait) => {
                        log::debug!("accessibility bus: {e}; retrying in {wait:?}");
                        std::thread::sleep(wait);
                        attempt += 1;
                    }
                    None => {
                        log::warn!("accessibility bus: {e}; screen readers get no trees");
                        return;
                    }
                },
            }
        }
    };
    if let Err(e) = std::thread::Builder::new()
        .name("a11y bus wait".into())
        .spawn(run)
    {
        log::warn!("a11y bus wait thread: {e}");
    }
}

/// Runs `restarted` on a thread of its own each time a reader turns on an accessibility bus that
/// was restarted since the host made its adapters. accesskit_unix joins the new bus then, but only
/// with its root: adapters it holds active keep their nodes on the old bus's connection, so the
/// host makes them again. Call once [`when_bus_ready`] has answered.
pub fn on_bus_restart(restarted: impl Fn() + Send + 'static) {
    let run = move || {
        if let Err(e) = follow_launcher(&restarted) {
            log::info!("accessibility bus watch: {e}; a restarted bus is not rejoined");
        }
    };
    if let Err(e) = std::thread::Builder::new()
        .name("a11y bus watch".into())
        .spawn(run)
    {
        log::warn!("a11y bus watch thread: {e}");
    }
}

/// How long after the reader's IsEnabled the adapters are made again: accesskit_unix hears the
/// same signal and joins the new bus first.
const REJOIN_AFTER: Duration = Duration::from_secs(1);

fn follow_launcher(restarted: &impl Fn()) -> zbus::Result<()> {
    let conn = crate::session()?;
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.a11y.Bus")
        .path("/org/a11y/bus")
        .interface("org.freedesktop.DBus.Properties")
        .member("PropertiesChanged")
        .add_arg("org.a11y.Status")
        .build()?;
    let signals = crate::wait(zbus::MessageStream::for_match_rule(rule, &conn, None))?;
    let owner = crate::wait(async {
        zbus::fdo::DBusProxy::new(&conn)
            .await?
            .get_name_owner("org.a11y.Bus".try_into()?)
            .await
    })
    .ok()
    .map(|o| o.to_string());
    let mut launcher = Launcher::new(owner);
    for msg in crate::blocking_iter(signals) {
        let msg = msg?;
        let header = msg.header();
        let Some(sender) = header.sender() else {
            continue;
        };
        let Ok((_, changed, _)) = msg.body().deserialize::<(
            String,
            std::collections::HashMap<String, zbus::OwnedVariant>,
            Vec<String>,
        )>() else {
            continue;
        };
        let enabled = changed
            .get("IsEnabled")
            .and_then(|v| bool::try_from(v).ok());
        if launcher.heard(sender.as_str(), enabled) {
            log::info!("accessibility bus restarted; making the adapters again");
            std::thread::sleep(REJOIN_AFTER);
            restarted();
        }
    }
    Ok(())
}

/// The accessibility bus launcher behind `org.a11y.Bus`, followed through its signals.
#[derive(Debug, Default)]
struct Launcher {
    /// The unique name of the launcher the adapters last joined.
    owner: Option<String>,
    /// Another launcher took the name since the adapters were made.
    stale: bool,
}

impl Launcher {
    fn new(owner: Option<String>) -> Launcher {
        Launcher {
            owner,
            stale: false,
        }
    }

    /// A PropertiesChanged on `org.a11y.Status` from `sender`, carrying IsEnabled `enabled` if
    /// it changed. True when the adapters are to be made again: a reader turned on a launcher
    /// that took the name after the one they joined.
    fn heard(&mut self, sender: &str, enabled: Option<bool>) -> bool {
        if self.owner.as_deref() != Some(sender) {
            self.stale |= self.owner.is_some();
            self.owner = Some(sender.to_string());
        }
        if enabled == Some(true) && self.stale {
            self.stale = false;
            return true;
        }
        false
    }
}

/// The wait before retry `attempt` (from 0) of the bus read: 250 ms doubling to 4 s, giving up
/// after about a minute.
pub fn retry_after(attempt: u32) -> Option<Duration> {
    const FIRST_MS: u64 = 250;
    const MOST_MS: u64 = 4_000;
    const ATTEMPTS: u32 = 20;
    (attempt < ATTEMPTS).then(|| Duration::from_millis((FIRST_MS << attempt.min(4)).min(MOST_MS)))
}

/// What accesskit_unix's first adapter goes through: `org.a11y.Status`'s properties on the session
/// bus, then the accessibility bus and its registry, each started on demand.
fn status() -> zbus::Result<()> {
    zbus::block_on(async {
        let session = zbus::Connection::session().await?;
        session
            .call_method(
                Some("org.a11y.Bus"),
                "/org/a11y/bus",
                Some("org.freedesktop.DBus.Properties"),
                "GetAll",
                &("org.a11y.Status",),
            )
            .await?;
        zbus::connection::Builder::address(a11y_address(&session).await?.as_str())
            .build()
            .await?
            .call_method(
                Some("org.a11y.atspi.Registry"),
                "/org/a11y/atspi/registry",
                Some("org.freedesktop.DBus.Peer"),
                "Ping",
                &(),
            )
            .await?;
        Ok(())
    })
}

pub struct KeyNote {
    pub pressed: bool,
    pub keysym: u32,
    pub keycode: u32,
    /// The X modifier mask: Shift 1, Lock 2, Control 4, Mod1 (Alt) 8, Mod2 (Num Lock) 16, Mod4 (Super) 64.
    pub mods: u32,
    pub time: u32,
    pub text: Zeroizing<String>,
    pub is_text: bool,
}

/// Reports keys to the registry from a thread of its own, since the calls block on the bus. The first
/// `spawn` connects; a host starts it once a screen reader first asks for a tree.
pub struct KeyEcho {
    tx: std::sync::mpsc::Sender<KeyNote>,
}

impl KeyEcho {
    pub fn spawn() -> KeyEcho {
        let (tx, rx) = std::sync::mpsc::channel::<KeyNote>();
        let run = move || {
            let mut conn = match a11y_bus() {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("accessibility bus: {e}; keys are not reported");
                    return;
                }
            };
            for n in rx {
                let event = (
                    u32::from(!n.pressed),
                    n.keysym as i32,
                    n.keycode as i32,
                    n.mods as i32,
                    n.time as i32,
                    n.text.as_str(),
                    n.is_text,
                );
                let notify = |conn: &zbus::Connection| {
                    crate::wait(conn.call_method(
                        Some("org.a11y.atspi.Registry"),
                        "/org/a11y/atspi/registry/deviceeventcontroller",
                        Some("org.a11y.atspi.DeviceEventController"),
                        "NotifyListenersSync",
                        &(event,),
                    ))
                };
                // A restarted bus leaves the old connection closed: join the new one and retry.
                let sent = notify(&conn).or_else(|e| match e {
                    zbus::Error::MethodError(..) | zbus::Error::FDO(_) => Err(e),
                    _ => {
                        conn = a11y_bus()?;
                        notify(&conn)
                    }
                });
                if let Err(e) = sent {
                    log::debug!("NotifyListenersSync: {e}");
                }
            }
        };
        if let Err(e) = std::thread::Builder::new()
            .name("a11y keys".into())
            .spawn(run)
        {
            log::warn!("a11y keys thread: {e}");
        }
        KeyEcho { tx }
    }

    pub fn send(&self, note: KeyNote) {
        let _ = self.tx.send(note);
    }
}

/// A connection to the session's accessibility bus, whose address the session bus's `org.a11y.Bus` hands out.
fn a11y_bus() -> zbus::Result<zbus::Connection> {
    let address =
        zbus::block_on(async { a11y_address(&zbus::Connection::session().await?).await })?;
    crate::connect(zbus::connection::Builder::address(address.as_str()))
}

async fn a11y_address(session: &zbus::Connection) -> zbus::Result<String> {
    let reply = session
        .call_method(
            Some("org.a11y.Bus"),
            "/org/a11y/bus",
            Some("org.a11y.Bus"),
            "GetAddress",
            &(),
        )
        .await?;
    reply.body().deserialize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reader_turning_on_a_restarted_launcher_remakes_the_adapters_once() {
        let mut l = Launcher::new(Some(":1.5".into()));
        assert!(!l.heard(":1.5", Some(false)), "the same launcher off");
        assert!(!l.heard(":1.5", Some(true)), "the same launcher on again");
        assert!(!l.heard(":1.9", Some(false)), "a new launcher, still off");
        assert!(l.heard(":1.9", Some(true)), "a reader turns the new one on");
        assert!(!l.heard(":1.9", Some(true)), "made again only once");
        assert!(l.heard(":1.12", Some(true)), "a second restart");
    }

    #[test]
    fn the_first_launcher_heard_without_a_known_owner_remakes_nothing() {
        let mut l = Launcher::new(None);
        assert!(!l.heard(":1.5", Some(true)));
        assert!(!l.heard(":1.5", None));
        assert!(l.heard(":1.7", Some(true)), "a later launcher");
    }

    #[test]
    fn retries_double_from_a_quarter_second_to_four_seconds() {
        let ms: Vec<u128> = (0..7)
            .map(|a| retry_after(a).unwrap().as_millis())
            .collect();
        assert_eq!(ms, [250, 500, 1000, 2000, 4000, 4000, 4000]);
    }

    #[test]
    fn retries_give_up_after_about_a_minute() {
        let total: Duration = (0..).map_while(retry_after).sum();
        assert_eq!(total, Duration::from_millis(67_750));
        assert_eq!(retry_after(20), None);
    }
}
