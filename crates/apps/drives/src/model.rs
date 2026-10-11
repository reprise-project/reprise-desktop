//! What the applet decides from UDisks2 snapshots, with no bus: which new
//! volumes it mounts or asks a passphrase for, the rows of the Safely
//! Remove Hardware menu, and the words of its balloons.

use std::path::Path;

use reprisefs::udisks::{Disk, Snapshot, Volume};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arrival {
    Mount(String),
    Unlock(String),
}

/// File systems whose own packages (e2fsprogs, xfsprogs) clear `HintAuto`
/// in a udev rule so nothing mounts them while every session is locked.
/// The applet leaves arrivals alone under the lock already, so it mounts these.
const HINT_CLEARED_UNDER_LOCK: &[&str] = &["ext2", "ext3", "ext4", "ext4dev", "xfs"];

/// Whether `v` is the user's own removable volume and fine to act on
/// without being asked. A cleartext volume carries its container's hints.
fn auto(snap: &Snapshot, v: &Volume) -> bool {
    snap.removable(v) && (v.auto || HINT_CLEARED_UNDER_LOCK.contains(&v.fs_type.as_str()))
}

/// What appeared between `prev` and `next` that the applet mounts or
/// unlocks on its own: a file system or a locked container that is new,
/// or whose file system or container side is new (a card put into a
/// reader already plugged in). With no `prev`, nothing: the drives
/// already there when the session starts are left as they are.
///
/// A cleartext volume is not one of them: the applet mounts it once its
/// own unlock returns. UDisks2 notes who unlocked it only then, and a
/// mount before that is asked for an administrator's password.
pub fn arrivals(prev: Option<&Snapshot>, next: &Snapshot) -> Vec<Arrival> {
    let Some(prev) = prev else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for v in &next.volumes {
        if !auto(next, v) {
            continue;
        }
        let before = prev.volume(&v.path);
        if v.filesystem
            && v.mounts.is_empty()
            && v.backing.is_none()
            && before.is_none_or(|p| !p.filesystem)
        {
            out.push(Arrival::Mount(v.path.clone()));
        }
        if v.encrypted && v.cleartext.is_none() && before.is_none_or(|p| !p.encrypted) {
            out.push(Arrival::Unlock(v.path.clone()));
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub disk: String,
    pub label: String,
}

fn mounted_on<'a>(snap: &'a Snapshot, disk: &Disk) -> Vec<&'a Volume> {
    snap.volumes_on(&disk.path)
        .into_iter()
        .filter(|v| !v.mounts.is_empty() && snap.removable(v))
        .collect()
}

/// A row per removable drive with something mounted, in drive order:
/// "Safely remove <drive> - <volume names>".
pub fn entries(snap: &Snapshot) -> Vec<Entry> {
    snap.disks
        .iter()
        .filter(|d| d.removable)
        .filter_map(|d| {
            let mounted = mounted_on(snap, d);
            if mounted.is_empty() {
                return None;
            }
            let names: Vec<String> = mounted.iter().map(|v| v.name(Some(d))).collect();
            Some(Entry {
                disk: d.path.clone(),
                label: format!("Safely remove {} - {}", d.name(), names.join(", ")),
            })
        })
        .collect()
}

pub fn volume_name(snap: &Snapshot, v: &Volume) -> String {
    v.name(snap.disk_of(v))
}

/// The name the passphrase box gives a locked container: its label, else
/// its drive's name, since a container's label is often empty.
pub fn container_name(snap: &Snapshot, volume: &str) -> String {
    let v = snap.volume(volume);
    let label = v.and_then(|v| v.label.clone());
    let disk = v.and_then(|v| snap.disk_of(v)).map(Disk::name);
    label.or(disk).unwrap_or_else(|| "Removable Disk".into())
}

pub fn mounted_balloon(name: &str, at: &Path) -> (String, String) {
    (
        name.to_string(),
        format!("Mounted at {}. Click here to open it.", at.display()),
    )
}

pub fn safe_balloon(disk: &str) -> (String, String) {
    (
        "Safe To Remove Hardware".into(),
        format!("The '{disk}' device can now be safely removed from the system."),
    )
}

pub fn busy_balloon(disk: &str, volumes: &str) -> (String, String) {
    (
        format!("Problem Ejecting {disk}"),
        format!("'{volumes}' is in use. Close any program or window using it, and then try again."),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn disk(path: &str, removable: bool) -> Disk {
        Disk {
            path: path.into(),
            vendor: "Kingston".into(),
            model: "DataTraveler".into(),
            removable,
            can_power_off: removable,
            ..Disk::default()
        }
    }

    fn fat(path: &str, disk: &str, mounted: bool) -> Volume {
        Volume {
            path: path.into(),
            device: format!("/dev/{path}"),
            disk: Some(disk.into()),
            label: Some("KINGSTON".into()),
            filesystem: true,
            mounts: if mounted {
                vec![PathBuf::from("/run/media/u/KINGSTON")]
            } else {
                Vec::new()
            },
            auto: true,
            ..Volume::default()
        }
    }

    fn snap(disks: Vec<Disk>, volumes: Vec<Volume>) -> Snapshot {
        Snapshot { disks, volumes }
    }

    #[test]
    fn the_first_snapshot_mounts_nothing() {
        let s = snap(vec![disk("d", true)], vec![fat("sdb1", "d", false)]);
        assert!(arrivals(None, &s).is_empty());
    }

    #[test]
    fn a_new_stick_is_mounted_and_an_internal_disk_is_not() {
        let before = snap(vec![], vec![]);
        let after = snap(
            vec![disk("d", true), disk("ssd", false)],
            vec![fat("sdb1", "d", false), fat("sda2", "ssd", false)],
        );
        assert_eq!(
            arrivals(Some(&before), &after),
            [Arrival::Mount("sdb1".into())]
        );
    }

    #[test]
    fn a_volume_the_user_unmounted_stays_unmounted() {
        let before = snap(vec![disk("d", true)], vec![fat("sdb1", "d", true)]);
        let after = snap(vec![disk("d", true)], vec![fat("sdb1", "d", false)]);
        assert!(arrivals(Some(&before), &after).is_empty());
    }

    #[test]
    fn a_file_system_found_after_the_block_mounts_then() {
        let mut bare = fat("sdb1", "d", false);
        bare.filesystem = false;
        let before = snap(vec![disk("d", true)], vec![bare]);
        let after = snap(vec![disk("d", true)], vec![fat("sdb1", "d", false)]);
        assert_eq!(
            arrivals(Some(&before), &after),
            [Arrival::Mount("sdb1".into())]
        );
    }

    #[test]
    fn hidden_or_manual_volumes_are_left_alone() {
        let before = snap(vec![], vec![]);
        let mut ignored = fat("sdb1", "d", false);
        ignored.ignore = true;
        let mut manual = fat("sdb2", "d", false);
        manual.auto = false;
        let after = snap(vec![disk("d", true)], vec![ignored, manual]);
        assert!(arrivals(Some(&before), &after).is_empty());
    }

    #[test]
    fn an_ext4_or_xfs_stick_mounts_though_its_auto_hint_is_cleared() {
        let before = snap(vec![], vec![]);
        let stick = |path: &str, fs_type: &str| Volume {
            fs_type: fs_type.into(),
            auto: false,
            ..fat(path, "d", false)
        };
        let after = snap(
            vec![disk("d", true)],
            vec![
                stick("sdb1", "ext4"),
                stick("sdb2", "xfs"),
                stick("sdb3", "vfat"),
            ],
        );
        assert_eq!(
            arrivals(Some(&before), &after),
            [Arrival::Mount("sdb1".into()), Arrival::Mount("sdb2".into())]
        );
    }

    #[test]
    fn a_locked_container_asks_once() {
        let luks = Volume {
            path: "sdc".into(),
            disk: Some("v".into()),
            encrypted: true,
            auto: true,
            ..Volume::default()
        };
        let before = snap(vec![], vec![]);
        let locked = snap(vec![disk("v", true)], vec![luks.clone()]);
        assert_eq!(
            arrivals(Some(&before), &locked),
            [Arrival::Unlock("sdc".into())]
        );
        let clear = Volume {
            path: "dm0".into(),
            disk: Some("v".into()),
            backing: Some("sdc".into()),
            filesystem: true,
            auto: true,
            ..Volume::default()
        };
        let unlocked = snap(
            vec![disk("v", true)],
            vec![
                Volume {
                    cleartext: Some("dm0".into()),
                    ..luks
                },
                clear,
            ],
        );
        assert!(
            arrivals(Some(&locked), &unlocked).is_empty(),
            "the container is not asked again, and its cleartext waits for the unlock to return"
        );
    }

    #[test]
    fn the_menu_has_a_row_per_removable_drive_with_something_mounted() {
        let mut second = fat("sdb2", "d", true);
        second.label = None;
        let s = snap(
            vec![disk("d", true), disk("e", true), disk("ssd", false)],
            vec![
                fat("sdb1", "d", true),
                second,
                fat("sdc1", "e", false),
                fat("sda2", "ssd", true),
            ],
        );
        assert_eq!(
            entries(&s),
            [Entry {
                disk: "d".into(),
                label: "Safely remove Kingston DataTraveler - KINGSTON, Removable Disk".into(),
            }]
        );
        assert!(entries(&snap(vec![], vec![])).is_empty());
    }
}
