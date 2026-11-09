//! Whether the session's desktop is locked, as its compositor reports it on
//! `org.freedesktop.ScreenSaver`: this computer's lock in show mode, the remote side's in lock mode.

use futures_util::StreamExt;
use std::time::Duration;
use tokio::sync::watch;
use zbus::Connection;
use zbus::connection::Builder;

const NAME: &str = "org.freedesktop.ScreenSaver";
const PATH: &str = "/org/freedesktop/ScreenSaver";
/// How soon a check that failed on this side, not on the owner's, is tried again.
const RETRY: Duration = Duration::from_secs(1);

/// Follows the desktop's lock from a thread of its own. It reads locked until the name's owner
/// is found to be the process serving the session's display and answers unlocked.
pub fn follow() -> watch::Receiver<bool> {
    follow_on(Builder::session(), crate::wayland::compositor_pid)
}

/// [`follow`] on the bus `builder` connects to, with `compositor` giving the compositor's pid.
fn follow_on(
    builder: Builder<'static>,
    compositor: impl FnMut() -> Result<u32, String> + Send + 'static,
) -> watch::Receiver<bool> {
    let (locked, rx) = watch::channel(true);
    let spawned = std::thread::Builder::new()
        .name("desktop-lock".into())
        .spawn(move || {
            let mut seen = Seen::new(locked);
            let ended = match reprisebus::connect(builder) {
                Ok(conn) => reprisebus::wait(run(&conn, compositor, &mut seen)),
                Err(e) => format!("no session bus: {e}"),
            };
            seen.set(Lock::Unknown(ended));
        });
    if let Err(e) = spawned {
        log::warn!("cannot follow the desktop's lock: {e}; the clipboard is not shared");
    }
    rx
}

#[derive(Clone, PartialEq, Debug)]
enum Lock {
    Locked,
    Unlocked,
    /// Why the lock cannot be told; the clipboard is held as if locked.
    Unknown(String),
}

/// The lock as published, logged once per change.
struct Seen {
    locked: watch::Sender<bool>,
    last: Option<Lock>,
}

impl Seen {
    fn new(locked: watch::Sender<bool>) -> Seen {
        Seen { locked, last: None }
    }

    fn set(&mut self, now: Lock) {
        if self.last.as_ref() == Some(&now) {
            return;
        }
        match &now {
            Lock::Locked => log::info!("the desktop locked"),
            Lock::Unlocked => log::info!("the desktop unlocked"),
            Lock::Unknown(why) => {
                log::warn!("the desktop's lock is unknown: {why}; the clipboard is not shared");
            }
        }
        self.locked.send_replace(now != Lock::Unlocked);
        self.last = Some(now);
    }
}

/// Follows the name until the bus connection ends; returns why it ended.
async fn run(
    conn: &Connection,
    mut compositor: impl FnMut() -> Result<u32, String>,
    seen: &mut Seen,
) -> String {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(NAME)
        .member("ActiveChanged")
        .path(PATH)
        .build()
        .map(Into::into);
    let rule = match rule {
        Ok(rule) => rule,
        Err(e) => return format!("ActiveChanged: {e}"),
    };
    let mut signals = reprisebus::signals(conn, NAME, vec![rule]).await;
    let runtime = match zruntime::SharedRuntime::current() {
        Ok(runtime) => runtime,
        Err(e) => return format!("no runtime: {e}"),
    };
    let mut trusted = None;
    let mut read_in = Some(Duration::ZERO);
    loop {
        let due = async {
            match read_in {
                Some(after) => runtime.sleep(after).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            signal = signals.next() => match signal {
                None => return "the session bus connection ended".into(),
                Some(reprisebus::Signal::OwnerChanged) => {
                    trusted = None;
                    read_in = Some(Duration::ZERO);
                }
                Some(reprisebus::Signal::Message(m)) => {
                    let sender = m.header().sender().map(|s| s.to_string());
                    if sender.is_some() && sender == trusted
                        && let Ok(active) = m.body().deserialize::<bool>()
                    {
                        seen.set(if active { Lock::Locked } else { Lock::Unlocked });
                    }
                }
            },
            () = due => {
                read_in = None;
                match read(conn, &mut compositor).await {
                    Ok((owner, active)) => {
                        trusted = Some(owner);
                        seen.set(if active { Lock::Locked } else { Lock::Unlocked });
                    }
                    Err(Unread { why, again }) => {
                        trusted = None;
                        seen.set(Lock::Unknown(why));
                        read_in = again.then_some(RETRY);
                    }
                }
            }
        }
    }
}

/// Why [`read`] could not tell; `again` when trying later may succeed without the name changing
/// hands.
struct Unread {
    why: String,
    again: bool,
}

/// The name's owner and its `GetActive`, once the owner is found to be the compositor.
async fn read(
    conn: &Connection,
    compositor: &mut impl FnMut() -> Result<u32, String>,
) -> Result<(String, bool), Unread> {
    let Some(owner) = reprisebus::name_owner(conn, NAME).await else {
        let why = format!("nothing owns {NAME}");
        return Err(Unread { why, again: false });
    };
    let pid = compositor().map_err(|e| Unread {
        why: format!("the compositor to check {NAME}'s owner against: {e}"),
        again: true,
    })?;
    if !reprisebus::peer::is_process(conn, &owner, pid).await {
        let why = format!("{NAME} is held by {owner}, not the compositor");
        return Err(Unread { why, again: false });
    }
    let reply = conn
        .call_method(Some(owner.as_str()), PATH, Some(NAME), "GetActive", &())
        .await;
    match reply.and_then(|m| m.body().deserialize::<bool>()) {
        Ok(active) => Ok((owner, active)),
        Err(e) => Err(Unread {
            why: format!("GetActive from {owner}: {e}"),
            again: true,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::tests::bus;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    use zbus::Connection;
    use zbus::object_server::SignalEmitter;

    struct Saver(Arc<AtomicBool>);

    #[zbus::interface(name = "org.freedesktop.ScreenSaver")]
    impl Saver {
        fn get_active(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }

        #[zbus(signal)]
        async fn active_changed(emitter: &SignalEmitter<'_>, active: bool) -> zbus::Result<()>;
    }

    /// A connection to `bus` owning the name, answering `active`.
    fn owner(bus: &str, active: bool) -> Connection {
        let saver = Saver(Arc::new(AtomicBool::new(active)));
        zbus::block_on(
            Builder::address(bus)
                .serve_at(PATH, saver)
                .name(NAME)
                .build(),
        )
        .unwrap()
    }

    fn signal(conn: &Connection, active: bool) {
        zbus::block_on(async {
            let emitter = SignalEmitter::new(conn, PATH).unwrap();
            Saver::active_changed(&emitter, active).await.unwrap();
        });
    }

    fn reads(rx: &watch::Receiver<bool>, want: bool) -> bool {
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            if *rx.borrow() == want {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    fn this_process() -> Result<u32, String> {
        Ok(std::process::id())
    }

    #[test]
    fn the_compositors_lock_is_followed_and_an_owner_leaving_holds_the_clipboard() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let first = owner(&bus.1, false);
        let rx = follow_on(Builder::address(bus.1.as_str()), this_process);
        assert!(reads(&rx, false), "GetActive is read");
        signal(&first, true);
        assert!(reads(&rx, true), "ActiveChanged is followed");
        signal(&first, false);
        assert!(reads(&rx, false));
        drop(first);
        assert!(reads(&rx, true), "no owner reads locked");
        let _later = owner(&bus.1, false);
        assert!(reads(&rx, false), "a later owner is read");
    }

    #[test]
    fn an_owner_that_is_not_the_compositor_reads_locked() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let compositor = Arc::new(AtomicU32::new(std::process::id()));
        let pid = compositor.clone();
        let _first = owner(&bus.1, false);
        let rx = follow_on(Builder::address(bus.1.as_str()), move || {
            Ok(pid.load(Ordering::SeqCst))
        });
        assert!(reads(&rx, false));
        compositor.store(1, Ordering::SeqCst);
        let _replacing = owner(&bus.1, false);
        assert!(reads(&rx, true), "an owner of another process");
    }

    #[test]
    fn the_lock_reads_locked_until_the_compositor_can_be_checked() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let _owner = owner(&bus.1, false);
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let rx = follow_on(Builder::address(bus.1.as_str()), move || {
            match counted.fetch_add(1, Ordering::SeqCst) {
                0 => Err("not up yet".into()),
                _ => Ok(std::process::id()),
            }
        });
        assert!(*rx.borrow(), "locked before the first read");
        assert!(reads(&rx, false), "checked again");
        assert!(calls.load(Ordering::SeqCst) >= 2);
    }
}
