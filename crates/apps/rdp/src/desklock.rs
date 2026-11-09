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
                    kept(crate::wayland::compositor_pid),
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

/// `read` until it gives a pid, then that pid: the server ends with the display, so the process
/// serving it can't change.
fn kept(mut read: impl FnMut() -> Result<u32, String>) -> impl FnMut() -> Result<u32, String> {
    let mut pid = None;
    move || {
        let pid = match pid {
            Some(pid) => pid,
            None => *pid.insert(read()?),
        };
        Ok(pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compositor_is_asked_until_it_gives_its_pid_and_then_not_again() {
        let mut calls = 0;
        let mut pid = kept(move || {
            calls += 1;
            match calls {
                1 => Err("not up yet".into()),
                n => Ok(40 + n),
            }
        });
        assert!(pid().is_err());
        assert_eq!(pid(), Ok(42));
        assert_eq!(pid(), Ok(42), "kept");
    }
}
