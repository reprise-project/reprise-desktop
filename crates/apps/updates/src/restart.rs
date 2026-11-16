//! A restart the running system needs that no install here asked for: a kernel package update
//! removed the running kernel's modules folder, or a package left `/run/reboot-required`. When
//! `/run/reboot-required.pkgs` names only Reprise, logging off and back on is enough.

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

use inotify::{EventMask, Inotify, WatchMask};

/// Where to look, the running kernel's release (`uname -r`) included.
#[derive(Clone, Debug)]
pub struct Paths {
    pub modules: PathBuf,
    pub release: String,
    pub flag: PathBuf,
    /// The packages that asked for the restart, one per line, beside `flag`.
    pub pkgs: PathBuf,
}

/// Why a restart is due.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Due {
    System,
    /// Only Reprise was upgraded: the session runs the old programs until it logs off.
    Reprise,
}

impl Paths {
    pub fn system() -> Option<Paths> {
        let release = std::fs::read_to_string("/proc/sys/kernel/osrelease").ok()?;
        Some(Paths {
            modules: "/usr/lib/modules".into(),
            release: release.trim().to_string(),
            flag: "/run/reboot-required".into(),
            pkgs: "/run/reboot-required.pkgs".into(),
        })
    }

    pub fn due(&self) -> Option<Due> {
        if modules_gone(&self.modules, &self.release) {
            return Some(Due::System);
        }
        if !self.flag.exists() {
            return None;
        }
        let pkgs = std::fs::read_to_string(&self.pkgs).unwrap_or_default();
        let mut named = pkgs.lines().map(str::trim).filter(|l| !l.is_empty());
        let first = named.next();
        match first == Some(PACKAGE) && named.all(|p| p == PACKAGE) {
            true => Some(Due::Reprise),
            false => Some(Due::System),
        }
    }
}

/// The name Reprise's packages write into `reboot-required.pkgs`.
const PACKAGE: &str = "reprise";

/// The release's folder is missing while another kernel's is there: a modules folder with no
/// kernel in it is a system that keeps none.
fn modules_gone(modules: &Path, release: &str) -> bool {
    !modules.join(release).is_dir()
        && std::fs::read_dir(modules)
            .is_ok_and(|mut d| d.any(|e| e.is_ok_and(|e| e.path().is_dir())))
}

/// Calls `changed` with the restart due, at once and then each time that changes; returns only
/// when inotify fails.
pub fn watch(paths: &Paths, mut changed: impl FnMut(Option<Due>)) -> io::Result<()> {
    let mut inotify = Inotify::init()?;
    // CLOSE_WRITE for a package list appended to in place.
    let mask = WatchMask::CREATE
        | WatchMask::DELETE
        | WatchMask::MOVED_FROM
        | WatchMask::MOVED_TO
        | WatchMask::CLOSE_WRITE;
    let modules = inotify.watches().add(&paths.modules, mask);
    if let Err(e) = &modules {
        log::info!("{} is not watched: {e}", paths.modules.display());
    }
    let names: Vec<_> = [&paths.flag, &paths.pkgs]
        .iter()
        .filter_map(|p| p.file_name().map(OsStr::to_os_string))
        .collect();
    if let Some(dir) = paths.flag.parent()
        && let Err(e) = inotify.watches().add(dir, mask)
    {
        log::info!("{} is not watched: {e}", dir.display());
    }
    let mut due = paths.due();
    changed(due);
    let mut buf = [0u8; 4096];
    loop {
        let events = match inotify.read_events_blocking(&mut buf) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        // Of the flag's folder (busy `/run`), only the flag's and the package list's names count.
        let relevant = events.into_iter().any(|e| {
            !e.mask.contains(EventMask::IGNORED)
                && (modules.as_ref().is_ok_and(|m| *m == e.wd)
                    || e.name.is_some_and(|n| names.iter().any(|m| m == n)))
        });
        if relevant && paths.due() != due {
            due = paths.due();
            changed(due);
        }
    }
}

pub fn boot_id() -> String {
    std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// The restart balloon's once-a-boot mark, kept across logins: `$XDG_STATE_HOME/reprise/restart-notice`
/// holds the boot id it was last shown in.
pub fn mark_path() -> PathBuf {
    reprisexdg::state_home()
        .unwrap_or_default()
        .join("reprise")
        .join("restart-notice")
}

/// Whether boot `boot` has not had the balloon yet; it has once this returns.
pub fn first_this_boot(mark: &Path, boot: &str) -> bool {
    if std::fs::read_to_string(mark).is_ok_and(|s| s.trim() == boot) {
        return false;
    }
    if let Some(dir) = mark.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(mark, format!("{boot}\n")) {
        log::warn!("cannot write {}: {e}", mark.display());
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("reprise-restart-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("modules")).unwrap();
        std::fs::create_dir_all(dir.join("run")).unwrap();
        dir
    }

    fn paths(dir: &Path) -> Paths {
        Paths {
            modules: dir.join("modules"),
            release: "6.17.1-1".into(),
            flag: dir.join("run/reboot-required"),
            pkgs: dir.join("run/reboot-required.pkgs"),
        }
    }

    #[test]
    fn a_restart_is_due_once_the_running_kernels_modules_are_gone() {
        let dir = scratch("due");
        let p = paths(&dir);
        assert_eq!(p.due(), None, "no kernel's modules at all");
        std::fs::create_dir(p.modules.join("6.17.1-1")).unwrap();
        assert_eq!(p.due(), None, "the running kernel's are there");
        std::fs::create_dir(p.modules.join("6.12.9-lts")).unwrap();
        assert_eq!(p.due(), None, "a second kernel installed beside it");
        std::fs::remove_dir(p.modules.join("6.17.1-1")).unwrap();
        assert_eq!(p.due(), Some(Due::System), "only another kernel's are left");
        std::fs::create_dir(p.modules.join("6.17.1-1")).unwrap();
        std::fs::write(&p.flag, "").unwrap();
        assert_eq!(p.due(), Some(Due::System), "the flag file");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_flag_only_reprise_asked_for_is_due_to_reprise() {
        let dir = scratch("pkgs");
        let p = paths(&dir);
        std::fs::create_dir(p.modules.join("6.17.1-1")).unwrap();
        std::fs::write(&p.flag, "").unwrap();
        std::fs::write(&p.pkgs, "reprise\n").unwrap();
        assert_eq!(p.due(), Some(Due::Reprise));
        std::fs::write(&p.pkgs, "reprise\nlinux-firmware\n").unwrap();
        assert_eq!(p.due(), Some(Due::System), "another package asked too");
        std::fs::write(&p.pkgs, "").unwrap();
        assert_eq!(p.due(), Some(Due::System), "no package named");
        std::fs::write(&p.pkgs, "reprise\n").unwrap();
        std::fs::remove_dir(p.modules.join("6.17.1-1")).unwrap();
        std::fs::create_dir(p.modules.join("6.17.2-1")).unwrap();
        assert_eq!(p.due(), Some(Due::System), "the kernel went too");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A kernel update empties the running release's folder, removes it, then makes the new one.
    #[test]
    fn the_watch_tells_of_a_kernel_update_and_a_flag_file() {
        let dir = scratch("watch");
        let p = paths(&dir);
        let old = p.modules.join("6.17.1-1");
        std::fs::create_dir_all(old.join("kernel")).unwrap();
        std::fs::write(old.join("kernel/a.ko"), "").unwrap();
        let (tx, rx) = channel();
        let watched = p.clone();
        std::thread::spawn(move || watch(&watched, |due| tx.send(due).unwrap()));
        let next = || rx.recv_timeout(Duration::from_secs(5));
        assert_eq!(next(), Ok(None));

        std::fs::write(dir.join("run/unrelated"), "").unwrap();
        std::fs::remove_dir_all(&old).unwrap();
        std::fs::create_dir(p.modules.join("6.17.2-1")).unwrap();
        assert_eq!(next(), Ok(Some(Due::System)));
        std::fs::create_dir(p.modules.join("6.17.2-1/kernel")).unwrap();
        std::fs::write(&p.flag, "").unwrap();
        std::fs::create_dir(&old).unwrap();
        std::fs::remove_file(&p.flag).unwrap();
        assert_eq!(next(), Ok(None), "the release is back and the flag gone");
        std::fs::write(&p.pkgs, "reprise\n").unwrap();
        std::fs::write(&p.flag, "").unwrap();
        assert_eq!(next(), Ok(Some(Due::Reprise)));
        std::fs::write(&p.pkgs, "reprise\nsystemd\n").unwrap();
        assert_eq!(next(), Ok(Some(Due::System)), "the package list grew");
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_balloon_is_due_once_a_boot() {
        let dir = scratch("mark");
        let mark = dir.join("state/reprise/restart-notice");
        assert!(first_this_boot(&mark, "boot-a"));
        assert!(
            !first_this_boot(&mark, "boot-a"),
            "a second login in that boot"
        );
        assert!(first_this_boot(&mark, "boot-b"), "after a restart");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
