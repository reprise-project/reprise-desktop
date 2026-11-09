//! Whether the session's desktop is locked, as its compositor reports it on
//! `org.freedesktop.ScreenSaver`: this computer's lock in show mode, the remote side's in lock mode.

use crate::Signal;
use futures_util::StreamExt;
use futures_util::future::{Either, select};
use std::pin::pin;
use std::time::Duration;
use zbus::Connection;

const NAME: &str = "org.freedesktop.ScreenSaver";
const PATH: &str = "/org/freedesktop/ScreenSaver";
/// How soon a check that failed on this side, not on the owner's, is tried again.
pub const RETRY: Duration = Duration::from_secs(1);

#[derive(Clone, PartialEq, Debug)]
pub enum Lock {
    Locked,
    Unlocked,
    /// Why the lock cannot be told.
    Unknown(String),
}

impl Lock {
    fn of(active: bool) -> Lock {
        if active { Lock::Locked } else { Lock::Unlocked }
    }
}

/// Follows the lock on `conn` until the connection ends and returns why. `report` hears every
/// change; the name's owner counts only once its process is `compositor`'s, until then Unknown.
pub async fn follow<F: Future<Output = ()>>(
    conn: &Connection,
    mut compositor: impl FnMut() -> Result<u32, String>,
    mut report: impl FnMut(Lock) -> F,
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
    let mut signals = crate::signals(conn, NAME, vec![rule]).await;
    let runtime = match zruntime::SharedRuntime::current() {
        Ok(runtime) => runtime,
        Err(e) => return format!("no runtime: {e}"),
    };
    let mut last = None;
    let mut trusted = None;
    let mut read_in = Some(Duration::ZERO);
    loop {
        let (runtime, after) = (&runtime, read_in);
        let due = pin!(async move {
            match after {
                Some(after) => runtime.sleep(after).await,
                None => std::future::pending().await,
            }
        });
        let now = match select(signals.next(), due).await {
            Either::Left((None, _)) => return "the session bus connection ended".into(),
            Either::Left((Some(Signal::OwnerChanged), _)) => {
                trusted = None;
                read_in = Some(Duration::ZERO);
                continue;
            }
            Either::Left((Some(Signal::Message(m)), _)) => {
                let sender = m.header().sender().map(|s| s.to_string());
                match m.body().deserialize::<bool>() {
                    Ok(active) if sender.is_some() && sender == trusted => Lock::of(active),
                    _ => continue,
                }
            }
            Either::Right(((), _)) => {
                read_in = None;
                match read(conn, &mut compositor).await {
                    Ok((owner, active)) => {
                        trusted = Some(owner);
                        Lock::of(active)
                    }
                    Err(Unread { why, again }) => {
                        trusted = None;
                        read_in = again.then_some(RETRY);
                        Lock::Unknown(why)
                    }
                }
            }
        };
        if last.as_ref() != Some(&now) {
            last = Some(now.clone());
            report(now).await;
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
    let Some(owner) = crate::name_owner(conn, NAME).await else {
        let why = format!("nothing owns {NAME}");
        return Err(Unread { why, again: false });
    };
    let pid = compositor().map_err(|e| Unread {
        why: format!("the compositor to check {NAME}'s owner against: {e}"),
        again: true,
    })?;
    match crate::peer::is_process(conn, &owner, pid).await {
        Ok(true) => {}
        Ok(false) => {
            let why = format!("{NAME} is held by {owner}, not the compositor");
            return Err(Unread { why, again: false });
        }
        Err(e) => {
            let why = format!("the process behind {owner}: {e}");
            return Err(Unread { why, again: true });
        }
    }
    let reply = conn
        .call_method(Some(owner.as_str()), PATH, Some(NAME), "GetActive", &())
        .await;
    match reply.and_then(|m| m.body().deserialize::<bool>()) {
        Ok(active) => Ok((owner, active)),
        Err(e) => Err(Unread {
            again: !lacks_get_active(&e),
            why: format!("GetActive from {owner}: {e}"),
        }),
    }
}

/// Whether `e` says the owner has no `GetActive`, as a compositor from before it doesn't; the
/// owner can't grow one without the name changing hands.
fn lacks_get_active(e: &zbus::Error) -> bool {
    use zbus::fdo::Error::{UnknownInterface, UnknownMethod};
    matches!(e, zbus::Error::FDO(e) if matches!(**e, UnknownMethod(_) | UnknownInterface(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peer::tests::bus;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;
    use zbus::connection::Builder;
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

    type Reports = Arc<Mutex<Vec<Lock>>>;

    /// What [`follow`] reports on `bus`, from a thread of its own.
    fn follow_on(
        bus: &str,
        compositor: impl FnMut() -> Result<u32, String> + Send + 'static,
    ) -> Reports {
        let reports = Reports::default();
        let out = reports.clone();
        let conn = crate::connect(Builder::address(bus)).unwrap();
        std::thread::spawn(move || {
            crate::wait(follow(&conn, compositor, |lock| {
                out.lock().unwrap().push(lock);
                std::future::ready(())
            }))
        });
        reports
    }

    fn reads(reports: &Reports, want: impl Fn(&Lock) -> bool) -> bool {
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            if reports.lock().unwrap().last().is_some_and(&want) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    fn unknown(lock: &Lock) -> bool {
        matches!(lock, Lock::Unknown(_))
    }

    fn this_process() -> Result<u32, String> {
        Ok(std::process::id())
    }

    #[test]
    fn the_compositors_lock_is_followed_and_an_owner_leaving_is_unknown() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let first = owner(&bus.1, false);
        let rx = follow_on(&bus.1, this_process);
        assert!(reads(&rx, |l| *l == Lock::Unlocked), "GetActive is read");
        signal(&first, true);
        assert!(reads(&rx, |l| *l == Lock::Locked), "ActiveChanged");
        signal(&first, false);
        assert!(reads(&rx, |l| *l == Lock::Unlocked));
        drop(first);
        assert!(reads(&rx, unknown), "no owner");
        let _later = owner(&bus.1, false);
        assert!(reads(&rx, |l| *l == Lock::Unlocked), "a later owner");
        let reports = rx.lock().unwrap().clone();
        assert!(
            reports.windows(2).all(|w| w[0] != w[1]),
            "only changes are reported: {reports:?}"
        );
    }

    #[test]
    fn an_owner_that_is_not_the_compositor_is_unknown() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let compositor = Arc::new(AtomicU32::new(std::process::id()));
        let pid = compositor.clone();
        let _first = owner(&bus.1, false);
        let rx = follow_on(&bus.1, move || Ok(pid.load(Ordering::SeqCst)));
        assert!(reads(&rx, |l| *l == Lock::Unlocked));
        compositor.store(1, Ordering::SeqCst);
        let _replacing = owner(&bus.1, false);
        assert!(reads(&rx, unknown), "an owner of another process");
        compositor.store(std::process::id(), Ordering::SeqCst);
        let _back = owner(&bus.1, true);
        assert!(
            reads(&rx, |l| *l == Lock::Locked),
            "read again once the name changes hands"
        );
    }

    struct OldSaver;

    #[zbus::interface(name = "org.freedesktop.ScreenSaver")]
    impl OldSaver {
        fn inhibit(&self, _app: String, _reason: String) -> u32 {
            1
        }
    }

    #[test]
    fn an_owner_without_get_active_is_asked_again_only_once_the_name_changes_hands() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let _old = zbus::block_on(
            Builder::address(bus.1.as_str())
                .serve_at(PATH, OldSaver)
                .name(NAME)
                .build(),
        )
        .unwrap();
        let reads_done = Arc::new(AtomicUsize::new(0));
        let counted = reads_done.clone();
        let rx = follow_on(&bus.1, move || {
            counted.fetch_add(1, Ordering::SeqCst);
            Ok(std::process::id())
        });
        assert!(reads(&rx, unknown));
        std::thread::sleep(RETRY * 2 + Duration::from_millis(500));
        assert_eq!(reads_done.load(Ordering::SeqCst), 1, "one read");
        let _new = owner(&bus.1, false);
        assert!(reads(&rx, |l| *l == Lock::Unlocked), "a new owner is read");
    }

    #[test]
    fn the_lock_is_unknown_until_the_compositor_can_be_checked() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let _owner = owner(&bus.1, false);
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let rx = follow_on(&bus.1, move || {
            match counted.fetch_add(1, Ordering::SeqCst) {
                0 => Err("not up yet".into()),
                _ => Ok(std::process::id()),
            }
        });
        assert!(reads(&rx, |l| *l == Lock::Unlocked), "checked again");
        assert!(unknown(&rx.lock().unwrap()[0]), "unknown before");
        assert!(calls.load(Ordering::SeqCst) >= 2);
    }
}
