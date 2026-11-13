//! The kernel's radio switches: read from `/sys/class/rfkill`, blocked, unblocked and followed
//! through `/dev/rfkill`, which the seat's user may open through systemd's uaccess rule.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

const SYSFS: &str = "/sys/class/rfkill";
const DEVICE: &str = "/dev/rfkill";

const OP_CHANGE_ALL: u8 = 3;

/// `RFKILL_IOC_NOINPUT`, `_IO('R', 1)`.
const IOC_NOINPUT: libc::c_ulong = 0x5201;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Wlan,
    Bluetooth,
    Wwan,
}

impl Kind {
    /// `enum rfkill_type`.
    fn code(self) -> u8 {
        match self {
            Kind::Wlan => 1,
            Kind::Bluetooth => 2,
            Kind::Wwan => 5,
        }
    }

    /// The kind a switch's sysfs `type` names; None for a radio that is not one of these.
    fn named(name: &str) -> Option<Kind> {
        match name {
            "wlan" => Some(Kind::Wlan),
            "bluetooth" => Some(Kind::Bluetooth),
            "wwan" => Some(Kind::Wwan),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Switch {
    pub kind: Kind,
    pub soft: bool,
    pub hard: bool,
}

/// The machine's Wi-Fi, Bluetooth and mobile broadband switches.
pub fn switches() -> Vec<Switch> {
    switches_in(Path::new(SYSFS))
}

fn switches_in(root: &Path) -> Vec<Switch> {
    let Ok(dir) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let read = |p: &Path, f: &str| std::fs::read_to_string(p.join(f)).unwrap_or_default();
    let mut found: Vec<(String, Switch)> = dir
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let kind = Kind::named(read(&p, "type").trim())?;
            Some((
                e.file_name().to_string_lossy().into_owned(),
                Switch {
                    kind,
                    soft: read(&p, "soft").trim() == "1",
                    hard: read(&p, "hard").trim() == "1",
                },
            ))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found.into_iter().map(|(_, s)| s).collect()
}

/// A `struct rfkill_event` in its first, 8-byte layout, which every kernel reads: idx (u32),
/// type, op, soft, hard. CHANGE_ALL ignores idx.
fn change_all(kind: Kind, blocked: bool) -> [u8; 8] {
    [0, 0, 0, 0, kind.code(), OP_CHANGE_ALL, blocked as u8, 0]
}

/// Soft-block or unblock every radio of `kind`. A hard block stays.
pub fn set_blocked(kind: Kind, blocked: bool) -> std::io::Result<()> {
    OpenOptions::new()
        .write(true)
        .open(DEVICE)?
        .write_all(&change_all(kind, blocked))
}

/// Call `changed` once per switch on opening, then each time one is added, removed or changed,
/// until it returns false. Blocks, so it runs on a thread of its own.
pub fn follow(mut changed: impl FnMut() -> bool) -> std::io::Result<()> {
    let mut device = File::open(DEVICE)?;
    let mut event = [0u8; 32];
    loop {
        if device.read(&mut event)? == 0 || !changed() {
            return Ok(());
        }
    }
}

/// Keep the kernel from turning the radios itself on the airplane-mode key while the returned
/// file stays open, so the desktop's own toggle is the only one.
pub fn take_key() -> std::io::Result<File> {
    let device = File::open(DEVICE)?;
    // SAFETY: the fd is open for the call, and NOINPUT takes no argument.
    let r = unsafe { libc::ioctl(std::os::fd::AsRawFd::as_raw_fd(&device), IOC_NOINPUT) };
    if r < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(device)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_all_names_the_type_and_the_block() {
        assert_eq!(change_all(Kind::Bluetooth, false), [0, 0, 0, 0, 2, 3, 0, 0]);
        assert_eq!(change_all(Kind::Wlan, true), [0, 0, 0, 0, 1, 3, 1, 0]);
        assert_eq!(change_all(Kind::Wwan, true), [0, 0, 0, 0, 5, 3, 1, 0]);
    }

    #[test]
    fn switches_are_read_from_sysfs_skipping_other_radios() {
        let root = tempfile::tempdir().unwrap();
        for (dir, kind, soft, hard) in [
            ("rfkill0", "wlan", "0", "0"),
            ("rfkill1", "bluetooth", "1", "0"),
            ("rfkill2", "nfc", "0", "0"),
            ("rfkill3", "wwan", "0", "1"),
        ] {
            let d = root.path().join(dir);
            std::fs::create_dir(&d).unwrap();
            std::fs::write(d.join("type"), format!("{kind}\n")).unwrap();
            std::fs::write(d.join("soft"), format!("{soft}\n")).unwrap();
            std::fs::write(d.join("hard"), format!("{hard}\n")).unwrap();
        }
        let s = |kind, soft, hard| Switch { kind, soft, hard };
        assert_eq!(
            switches_in(root.path()),
            [
                s(Kind::Wlan, false, false),
                s(Kind::Bluetooth, true, false),
                s(Kind::Wwan, false, true),
            ]
        );
        assert_eq!(switches_in(&root.path().join("missing")), []);
    }
}
