//! Whole disks: which block devices are disks (`/sys/block`) and their counters
//! (`/proc/diskstats`).

use std::collections::HashMap;
use std::path::Path;

/// Bytes per sector in `diskstats` and `size`, whatever the disk's own sector size.
pub const SECTOR: u64 = 512;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Disk {
    /// The kernel's name: `nvme0n1`, `sda`, `vda`.
    pub name: String,
    pub model: String,
    pub bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stat {
    pub read_sectors: u64,
    pub written_sectors: u64,
    /// Milliseconds with I/O in flight.
    pub busy_ms: u64,
}

/// Whether block device `name` is a whole disk: backed by a device (not a partition, mapper or
/// software RAM disk), holding something, and neither an optical drive nor an eMMC's boot or
/// replay-protected area.
pub fn whole(name: &str, has_device: bool, sectors: u64) -> bool {
    let emmc_area = name.starts_with("mmcblk") && (name.contains("boot") || name.ends_with("rpmb"));
    has_device
        && sectors > 0
        && !emmc_area
        && !["loop", "ram", "zram", "sr"]
            .iter()
            .any(|p| name.starts_with(p))
}

/// The whole disks under `root` (`/` on a live system), by name.
pub fn scan(root: &Path) -> Vec<Disk> {
    let block = root.join("sys/block");
    let Ok(dir) = std::fs::read_dir(&block) else {
        return Vec::new();
    };
    let mut out: Vec<Disk> = dir
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let at = block.join(&name);
            let sectors: u64 = read_trim(&at.join("size"))?.parse().ok()?;
            if !whole(&name, at.join("device").exists(), sectors) {
                return None;
            }
            let model = ["device/model", "device/name"]
                .iter()
                .find_map(|f| read_trim(&at.join(f)).filter(|s| !s.is_empty()))
                .unwrap_or_default();
            Some(Disk {
                name,
                model,
                bytes: sectors * SECTOR,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Every device's counters in `diskstats` text, by name.
pub fn parse_diskstats(s: &str) -> HashMap<String, Stat> {
    s.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let n = |i: usize| f.get(i)?.parse::<u64>().ok();
            Some((
                f.get(2)?.to_string(),
                Stat {
                    read_sectors: n(5)?,
                    written_sectors: n(9)?,
                    busy_ms: n(12)?,
                },
            ))
        })
        .collect()
}

pub fn read_stats(root: &Path) -> HashMap<String, Stat> {
    std::fs::read_to_string(root.join("proc/diskstats"))
        .map(|s| parse_diskstats(&s))
        .unwrap_or_default()
}

pub(crate) fn read_trim(p: &Path) -> Option<String> {
    Some(std::fs::read_to_string(p).ok()?.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diskstats_reads_sectors_and_busy_time_per_device() {
        let s = " 259       0 nvme0n1 4595064 261606 365914559 5687675 8882645 2974645 \
                 1151529675 38947798 0 3251644 44906566 0 0 0 0 109429 271092\n   \
                 8       1 sda1 41 0 3344 19 0 0 0 0 0 13 19\n";
        let got = parse_diskstats(s);
        assert_eq!(
            got["nvme0n1"],
            Stat {
                read_sectors: 365_914_559,
                written_sectors: 1_151_529_675,
                busy_ms: 3_251_644,
            }
        );
        assert_eq!(
            got["sda1"].read_sectors, 3344,
            "a kernel with fewer fields still reads"
        );
    }

    #[test]
    fn partitions_mappers_and_ram_disks_are_not_whole_disks() {
        let root = tempfile::tempdir().unwrap();
        let block = root.path().join("sys/block");
        let disk = |name: &str, device: bool, size: u64, model: Option<&str>| {
            let d = block.join(name);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("size"), format!("{size}\n")).unwrap();
            if device {
                std::fs::create_dir(d.join("device")).unwrap();
                if let Some(m) = model {
                    std::fs::write(d.join("device/model"), format!("{m}   \n")).unwrap();
                }
            }
        };
        disk(
            "nvme0n1",
            true,
            2_000_409_264,
            Some("SKHynix_HFS001TDE9X081N"),
        );
        disk("vda", true, 41_943_040, None);
        disk("sr0", true, 2_097_152, Some("QEMU DVD-ROM"));
        disk("dm-0", false, 1_996_178_030, None);
        disk("zram0", true, 57_088_000, None);
        disk("loop0", false, 100, None);
        disk("mmcblk0", true, 122_126_336, None);
        disk("mmcblk0boot0", true, 8192, None);
        let got = scan(root.path());
        let names: Vec<&str> = got.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            names,
            ["mmcblk0", "nvme0n1", "vda"],
            "optical drives and an eMMC's boot area are left out too"
        );
        assert_eq!(got[1].model, "SKHynix_HFS001TDE9X081N");
        assert_eq!(got[1].bytes, 2_000_409_264 * 512);
        assert_eq!(got[2].model, "", "no model file");
    }
}
