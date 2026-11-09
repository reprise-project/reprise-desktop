//! Whether the session's desktop is locked, for the clipboard; [`reprisebus::desklock`] says
//! which lock that is.

use reprisebus::desklock::Lock;
use tokio::sync::watch;
use zbus::connection::Builder;

/// Follows the desktop's lock from a thread of its own. It reads locked until the name's owner
/// is found to be the process serving the session's display and answers unlocked.
pub fn follow() -> watch::Receiver<bool> {
    let (locked, rx) = watch::channel(true);
    let spawned = std::thread::Builder::new()
        .name("desktop-lock".into())
        .spawn(move || {
            let report = |lock| {
                publish(&locked, lock);
                std::future::ready(())
            };
            let ended = match reprisebus::connect(Builder::session()) {
                Ok(conn) => reprisebus::wait(reprisebus::desklock::follow(
                    &conn,
                    crate::wayland::compositor_pid,
                    report,
                )),
                Err(e) => format!("no session bus: {e}"),
            };
            publish(&locked, Lock::Unknown(ended));
        });
    if let Err(e) = spawned {
        log::warn!("cannot follow the desktop's lock: {e}; the clipboard is not shared");
    }
    rx
}

fn publish(locked: &watch::Sender<bool>, now: Lock) {
    match &now {
        Lock::Locked => log::info!("the desktop locked"),
        Lock::Unlocked => log::info!("the desktop unlocked"),
        Lock::Unknown(why) => {
            log::warn!("the desktop's lock is unknown: {why}; the clipboard is not shared");
        }
    }
    locked.send_replace(now != Lock::Unlocked);
}
