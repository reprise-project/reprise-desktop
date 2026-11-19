//! Keys reported to the AT-SPI registry for a screen reader, the wait for the accessibility bus
//! before a host's first AccessKit adapter ([`when_bus_ready`]), and the watch for its restart
//! ([`on_bus_restart`]).
//!
//! A screen reader under Wayland cannot see the keyboard: Orca learns of keys from the toolkit of
//! whatever has the keyboard, which reports each one to the registry
//! (`DeviceEventController.NotifyListenersSync` on the accessibility bus). It needs them to know that a
//! caret or highlight moved because of an arrow key, and so to say the new line or row.

use std::time::Duration;

use futures_util::StreamExt;

use zeroize::Zeroizing;

/// Runs `ready` on a thread of its own once the accessibility bus answers ([`status`]), however
/// late ([`wait_for_bus`]). accesskit_unix connects once, when a process makes its first adapter:
/// a call failing while the bus is still starting (a session restart racing the old session's stop
/// job) leaves the process unseen by readers for good, so a host makes its first adapter only after
/// `ready`. Without a session bus there is nothing to wait on, and `ready` never runs.
pub fn when_bus_ready(ready: impl FnOnce() + Send + 'static) {
    let run = move || match crate::session() {
        Ok(session) => wait_for_bus(&session, ready),
        Err(e) => log::warn!("session bus: {e}; screen readers get no trees"),
    };
    if let Err(e) = std::thread::Builder::new()
        .name("a11y bus wait".into())
        .spawn(run)
    {
        log::warn!("a11y bus wait thread: {e}");
    }
}

const LAUNCHER: &str = "org.a11y.Bus";

/// Reads the accessibility bus on `session` until it answers, then runs `ready`; returns without
/// it when the session bus closes first. With no launcher on the bus and none to start, the
/// read waits asleep for one to take `org.a11y.Bus`. A launcher there that fails to start or
/// answer is read again on [`retry_after`]'s timer, and on each new launcher.
fn wait_for_bus(session: &zbus::Connection, ready: impl FnOnce()) {
    // Subscribed before the first read, so a launcher that comes during it is seen.
    let mut launchers = match crate::wait(launchers(session)) {
        Ok(l) => l,
        Err(e) => {
            log::warn!("accessibility bus: cannot follow {LAUNCHER}: {e}");
            return;
        }
    };
    let mut attempt = 0;
    loop {
        let retry = match status(session) {
            Ok(()) => return ready(),
            Err(e) if absent(session, &e) => {
                log::info!("accessibility bus: {e}; waiting for a launcher");
                None
            }
            Err(e) => {
                let retry = retry_after(attempt);
                attempt += 1;
                match retry {
                    Some(wait) => log::debug!("accessibility bus: {e}; retrying in {wait:?}"),
                    None => log::info!("accessibility bus: {e}; waiting for a new launcher"),
                }
                retry
            }
        };
        match crate::wait(woken(&mut launchers, retry)) {
            Woke::Launcher(name) => {
                log::info!("accessibility bus: {LAUNCHER} taken by {name}");
                attempt = 0;
            }
            Woke::Retry => {}
            Woke::Closed => {
                log::info!("session bus closed; screen readers get no trees");
                return;
            }
        }
    }
}

/// Whether a read failing with `e` found no launcher and none to start: nothing owns
/// `org.a11y.Bus` and the session bus has no service file for it.
fn absent(session: &zbus::Connection, e: &zbus::Error) -> bool {
    crate::error::name(e).is_some_and(|n| n == "org.freedesktop.DBus.Error.ServiceUnknown")
        && crate::wait(crate::name_owner(session, LAUNCHER)).is_none()
}

enum Woke {
    Launcher(String),
    Retry,
    Closed,
}

/// The next launcher in `launchers`, or [`Woke::Retry`] after `retry` when one is given.
async fn woken(
    launchers: &mut (impl futures_util::Stream<Item = String> + Unpin),
    retry: Option<Duration>,
) -> Woke {
    let launcher = async {
        match launchers.next().await {
            Some(name) => Woke::Launcher(name),
            None => Woke::Closed,
        }
    };
    let timer = async {
        match retry {
            Some(wait) => {
                async_io::Timer::after(wait).await;
                Woke::Retry
            }
            None => std::future::pending().await,
        }
    };
    futures_lite::future::or(launcher, timer).await
}

/// The unique names taking `org.a11y.Bus` on `session`, as the bus announces them.
async fn launchers(
    session: &zbus::Connection,
) -> zbus::Result<impl futures_util::Stream<Item = String> + Unpin> {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(crate::DBUS)
        .path(crate::DBUS_PATH)
        .interface(crate::DBUS)
        .member("NameOwnerChanged")
        .add_arg(LAUNCHER)
        .build()?;
    let changes = zbus::MessageStream::for_match_rule(rule, session, None).await?;
    Ok(changes.filter_map(|m| {
        let taken = match m.ok().and_then(|m| crate::owner_item(m, LAUNCHER)) {
            Some(crate::Item::Owner(owner)) => owner,
            _ => None,
        };
        futures_util::future::ready(taken)
    }))
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

/// The wait before timed retry `attempt` (from 0) of a read that found a launcher failing to
/// start or answer: 250 ms doubling to 4 s, none after about a minute.
pub fn retry_after(attempt: u32) -> Option<Duration> {
    const FIRST_MS: u64 = 250;
    const MOST_MS: u64 = 4_000;
    const ATTEMPTS: u32 = 20;
    (attempt < ATTEMPTS).then(|| Duration::from_millis((FIRST_MS << attempt.min(4)).min(MOST_MS)))
}

/// What accesskit_unix's first adapter goes through: `org.a11y.Status`'s properties on the session
/// bus, then the accessibility bus and its registry, each started on demand.
fn status(session: &zbus::Connection) -> zbus::Result<()> {
    zbus::block_on(async {
        session
            .call_method(
                Some("org.a11y.Bus"),
                "/org/a11y/bus",
                Some("org.freedesktop.DBus.Properties"),
                "GetAll",
                &("org.a11y.Status",),
            )
            .await?;
        zbus::connection::Builder::address(a11y_address(session).await?.as_str())
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
                let sent = notify(&conn).or_else(|e| {
                    if crate::error::name(&e).is_some() {
                        return Err(e);
                    }
                    conn = a11y_bus()?;
                    notify(&conn)
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
    use crate::testbus::{Bus, CEILING, bus_with_no_services, connect, serve};
    use std::sync::mpsc::{Receiver, Sender, channel};

    struct FakeStatus;

    #[zbus::interface(name = "org.a11y.Status")]
    impl FakeStatus {
        #[zbus(property)]
        fn is_enabled(&self) -> bool {
            true
        }
    }

    /// A launcher handing out `address`, or with `hang_up`, one that reports the call there and
    /// never answers it.
    struct FakeLauncher {
        address: String,
        hang_up: Option<Sender<()>>,
    }

    #[zbus::interface(name = "org.a11y.Bus")]
    impl FakeLauncher {
        async fn get_address(&self) -> String {
            if let Some(asked) = &self.hang_up {
                let _ = asked.send(());
                std::future::pending::<()>().await;
            }
            self.address.clone()
        }
    }

    struct FakeRegistry;

    #[zbus::interface(name = "org.a11y.atspi.Registry")]
    impl FakeRegistry {}

    /// A launcher owning `org.a11y.Bus` on the session bus at `session`, handing out `a11y`.
    fn launcher(session: &str, a11y: &str, hang_up: Option<Sender<()>>) -> zbus::Connection {
        zbus::block_on(async {
            let conn = connect(session).await;
            serve(&conn, "/org/a11y/bus", FakeStatus).await;
            let address = a11y.to_string();
            serve(&conn, "/org/a11y/bus", FakeLauncher { address, hang_up }).await;
            conn.request_name("org.a11y.Bus").await.unwrap();
            conn
        })
    }

    /// The registry on the accessibility bus `a11y`.
    fn registry(a11y: &Bus) -> zbus::Connection {
        zbus::block_on(async {
            let conn = connect(&a11y.1).await;
            serve(&conn, "/org/a11y/atspi/registry", FakeRegistry).await;
            conn.request_name("org.a11y.atspi.Registry").await.unwrap();
            conn
        })
    }

    /// A [`wait_for_bus`] on a thread of its own: its thread id, and "ready" when it runs `ready`,
    /// then "ended" when it returns.
    struct Waiting {
        tid: libc::pid_t,
        heard: Receiver<&'static str>,
    }

    fn start_wait(session: &str) -> Waiting {
        let conn = zbus::block_on(connect(session));
        let (tx, heard) = channel();
        let (tid_tx, tid) = channel();
        std::thread::spawn(move || {
            // SAFETY: gettid takes nothing and cannot fail.
            let _ = tid_tx.send(unsafe { libc::gettid() });
            wait_for_bus(&conn, || {
                let _ = tx.send("ready");
            });
            let _ = tx.send("ended");
        });
        Waiting {
            tid: tid.recv().unwrap(),
            heard,
        }
    }

    /// How often thread `tid` of this process has gone to sleep.
    fn sleeps(tid: libc::pid_t) -> u64 {
        let status = std::fs::read_to_string(format!("/proc/self/task/{tid}/status")).unwrap();
        status
            .lines()
            .find_map(|l| l.strip_prefix("voluntary_ctxt_switches:"))
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    /// Long enough for the wait's first read of the bus to fail.
    const SETTLE: Duration = Duration::from_millis(500);

    /// How long the accessibility bus stays away before it comes.
    const LATE: Duration = Duration::from_secs(2);

    #[test]
    fn the_wait_sleeps_while_no_accessibility_bus_is_on_the_session_bus() {
        let Some(session) = bus_with_no_services() else {
            return;
        };
        let wait = start_wait(&session.1);
        std::thread::sleep(SETTLE);
        let before = sleeps(wait.tid);
        std::thread::sleep(Duration::from_secs(3));
        assert_eq!(sleeps(wait.tid) - before, 0, "wakes over 3 s");
        assert!(wait.heard.try_recv().is_err(), "still waiting");
        drop(session);
        assert_eq!(
            wait.heard.recv_timeout(CEILING),
            Ok("ended"),
            "the wait ends with the session bus"
        );
    }

    #[test]
    fn a_bus_already_on_the_session_bus_is_joined_at_once() {
        let (Some(session), Some(a11y)) = (bus_with_no_services(), bus_with_no_services()) else {
            return;
        };
        let _registry = registry(&a11y);
        let _launcher = launcher(&session.1, &a11y.1, None);
        let wait = start_wait(&session.1);
        assert_eq!(wait.heard.recv_timeout(CEILING), Ok("ready"));
    }

    #[test]
    fn an_accessibility_bus_that_comes_after_the_wait_began_is_joined() {
        let (Some(session), Some(a11y)) = (bus_with_no_services(), bus_with_no_services()) else {
            return;
        };
        let wait = start_wait(&session.1);
        std::thread::sleep(LATE);
        assert!(wait.heard.try_recv().is_err(), "nothing to join yet");
        let _registry = registry(&a11y);
        let _launcher = launcher(&session.1, &a11y.1, None);
        assert_eq!(wait.heard.recv_timeout(CEILING), Ok("ready"));
    }

    #[test]
    fn a_launcher_gone_before_it_gave_the_address_is_waited_past_to_the_next() {
        let (Some(session), Some(a11y)) = (bus_with_no_services(), bus_with_no_services()) else {
            return;
        };
        let _registry = registry(&a11y);
        let wait = start_wait(&session.1);
        std::thread::sleep(SETTLE);
        let (asked_tx, asked) = channel();
        let first = launcher(&session.1, &a11y.1, Some(asked_tx));
        asked
            .recv_timeout(CEILING)
            .expect("the first launcher is asked");
        zbus::block_on(first.close()).unwrap();
        assert!(
            wait.heard.recv_timeout(SETTLE).is_err(),
            "nothing joined on a gone launcher"
        );
        let _second = launcher(&session.1, &a11y.1, None);
        assert_eq!(wait.heard.recv_timeout(CEILING), Ok("ready"));
    }

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
    fn timed_retries_double_from_a_quarter_second_to_four_seconds() {
        let ms: Vec<u128> = (0..7)
            .map(|a| retry_after(a).unwrap().as_millis())
            .collect();
        assert_eq!(ms, [250, 500, 1000, 2000, 4000, 4000, 4000]);
    }

    #[test]
    fn timed_retries_stop_after_about_a_minute() {
        let total: Duration = (0..).map_while(retry_after).sum();
        assert_eq!(total, Duration::from_millis(67_750));
        assert_eq!(retry_after(20), None);
    }

    #[test]
    fn a_launcher_whose_registry_is_still_starting_is_read_again() {
        let (Some(session), Some(a11y)) = (bus_with_no_services(), bus_with_no_services()) else {
            return;
        };
        let _launcher = launcher(&session.1, &a11y.1, None);
        let wait = start_wait(&session.1);
        std::thread::sleep(SETTLE);
        assert!(wait.heard.try_recv().is_err(), "no registry to ping yet");
        let _registry = registry(&a11y);
        assert_eq!(wait.heard.recv_timeout(CEILING), Ok("ready"));
    }
}
