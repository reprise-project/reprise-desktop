//! Wakes the daemon when an account file is written, so group look-ups see the change.

use std::ffi::OsStr;
use std::io;
use std::path::Path;

use inotify::{EventMask, Inotify, WatchMask};

/// The files in `/etc` whose writes can change a user's groups: members and primary gids.
pub const FILES: &[&str] = &["group", "passwd"];

/// Calls `written` from a thread after a batch of events holds a write or replacement of one of
/// `files` in `dir`, or the queue overflowed, until it returns false.
pub fn follow(
    dir: &Path,
    files: &'static [&'static str],
    written: impl Fn() -> bool + Send + 'static,
) -> io::Result<()> {
    let inotify = Inotify::init()?;
    // The directory, not the files: shadow-utils and systemd-sysusers replace them by rename.
    inotify
        .watches()
        .add(dir, WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO)?;
    std::thread::Builder::new()
        .name("accounts".into())
        .spawn(move || {
            if let Err(e) = watch(inotify, files, written) {
                log::warn!(
                    "stopped watching the account files ({e}); group changes reach the front's \
                     list only at a link, a reload, a password set or a disagreeing look-up"
                );
            }
        })?;
    Ok(())
}

fn watch(mut inotify: Inotify, files: &[&str], written: impl Fn() -> bool) -> io::Result<()> {
    let mut buf = [0u8; 4096];
    loop {
        let events = match inotify.read_events_blocking(&mut buf) {
            Ok(events) => events,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        let (mut changed, mut removed) = (false, false);
        for e in events {
            removed |= e.mask.contains(EventMask::IGNORED);
            changed |= e.mask.contains(EventMask::Q_OVERFLOW)
                || e.name
                    .is_some_and(|n| files.iter().any(|f| n == OsStr::new(f)));
        }
        if changed && !written() {
            return Ok(());
        }
        if removed {
            return Err(io::Error::other("the watch was removed"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(5);
    const QUIET: Duration = Duration::from_millis(300);

    fn watched() -> (tempfile::TempDir, mpsc::Receiver<()>) {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        follow(dir.path(), &["group"], move || tx.send(()).is_ok()).unwrap();
        (dir, rx)
    }

    #[test]
    fn a_group_file_replaced_by_rename_wakes_once() {
        let (dir, rx) = watched();
        std::fs::write(dir.path().join("hosts"), "x").unwrap();
        std::fs::write(dir.path().join("group-"), "x").unwrap();
        std::fs::write(dir.path().join("group+"), "x").unwrap();
        std::fs::rename(dir.path().join("group+"), dir.path().join("group")).unwrap();
        assert_eq!(rx.recv_timeout(WAIT), Ok(()));
        assert!(
            rx.recv_timeout(QUIET).is_err(),
            "hosts, group- and group+ wake nothing"
        );
    }

    #[test]
    fn a_group_file_written_in_place_wakes() {
        let (dir, rx) = watched();
        std::fs::write(dir.path().join("group"), "x").unwrap();
        assert_eq!(rx.recv_timeout(WAIT), Ok(()));
    }

    #[test]
    fn a_removed_watch_ends_the_watcher() {
        let (dir, rx) = watched();
        dir.close().unwrap();
        assert_eq!(
            rx.recv_timeout(WAIT),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );
    }
}
