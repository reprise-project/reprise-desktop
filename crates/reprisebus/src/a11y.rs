//! Keys reported to the AT-SPI registry for a screen reader, and the wait for the accessibility bus
//! before a host's first AccessKit adapter ([`when_bus_ready`]).
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
    let session = zbus::blocking::Connection::session()?;
    session.call_method(
        Some("org.a11y.Bus"),
        "/org/a11y/bus",
        Some("org.freedesktop.DBus.Properties"),
        "GetAll",
        &("org.a11y.Status",),
    )?;
    a11y_bus_of(&session)?.call_method(
        Some("org.a11y.atspi.Registry"),
        "/org/a11y/atspi/registry",
        Some("org.freedesktop.DBus.Peer"),
        "Ping",
        &(),
    )?;
    Ok(())
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
            let conn = match a11y_bus() {
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
                if let Err(e) = conn.call_method(
                    Some("org.a11y.atspi.Registry"),
                    "/org/a11y/atspi/registry/deviceeventcontroller",
                    Some("org.a11y.atspi.DeviceEventController"),
                    "NotifyListenersSync",
                    &(event,),
                ) {
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
fn a11y_bus() -> zbus::Result<zbus::blocking::Connection> {
    a11y_bus_of(&zbus::blocking::Connection::session()?)
}

fn a11y_bus_of(session: &zbus::blocking::Connection) -> zbus::Result<zbus::blocking::Connection> {
    let reply = session.call_method(
        Some("org.a11y.Bus"),
        "/org/a11y/bus",
        Some("org.a11y.Bus"),
        "GetAddress",
        &(),
    )?;
    let address: String = reply.body().deserialize()?;
    zbus::blocking::connection::Builder::address(address.as_str())?.build()
}

#[cfg(test)]
mod tests {
    use super::*;

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
