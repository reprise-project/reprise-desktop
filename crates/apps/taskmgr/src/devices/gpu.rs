//! GPUs: one per DRM card, named from the PCI ID list as About Reprise names them, with AMD's
//! own usage counters.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::disk::read_trim;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    /// `card1`.
    pub card: String,
    /// The device directory, resolved.
    pub dev: PathBuf,
    /// Its PCI address (`0000:07:00.0`), None off the PCI bus.
    pub pci: Option<String>,
    pub driver: String,
    pub name: String,
}

/// Names cards from the PCI ID list, read once and kept per id.
#[derive(Debug, Default)]
pub struct Names {
    ids: Option<String>,
    known: HashMap<(u16, u16), Option<String>>,
}

impl Names {
    fn name(&mut self, root: &Path, v: u16, d: u16) -> Option<String> {
        if let Some(n) = self.known.get(&(v, d)) {
            return n.clone();
        }
        let ids = self.ids.get_or_insert_with(|| {
            ["usr/share/hwdata/pci.ids", "usr/share/misc/pci.ids"]
                .iter()
                .find_map(|p| std::fs::read_to_string(root.join(p)).ok())
                .unwrap_or_default()
        });
        let n = reprisefs::sysinfo::pci_name(ids, v, d);
        self.known.insert((v, d), n.clone());
        n
    }
}

fn hex(p: &Path) -> Option<u16> {
    u16::from_str_radix(read_trim(p)?.trim_start_matches("0x"), 16).ok()
}

/// The PCI device `dev` is, or sits on (a virtio GPU's transport).
fn pci_of(dev: &Path) -> Option<&Path> {
    [Some(dev), dev.parent()].into_iter().flatten().find(|p| {
        std::fs::read_link(p.join("subsystem"))
            .is_ok_and(|s| s.file_name().is_some_and(|n| n == "pci"))
    })
}

/// The cards under `root` (`/` on a live system), by number; the firmware's framebuffer, which
/// gives way to the real driver, is left out.
pub fn scan(root: &Path, names: &mut Names) -> Vec<Card> {
    let class = root.join("sys/class/drm");
    let Ok(dir) = std::fs::read_dir(&class) else {
        return Vec::new();
    };
    let mut cards: Vec<(u32, Card)> = dir
        .flatten()
        .filter_map(|e| {
            let card = e.file_name().into_string().ok()?;
            let n: u32 = card.strip_prefix("card")?.parse().ok()?;
            let dev = std::fs::canonicalize(class.join(&card).join("device")).ok()?;
            let driver = std::fs::read_link(dev.join("driver"))
                .ok()
                .and_then(|p| Some(p.file_name()?.to_str()?.to_string()))
                .unwrap_or_default();
            if driver == "simple-framebuffer" {
                return None;
            }
            let on_pci = pci_of(&dev);
            let pci = on_pci.and_then(|p| Some(p.file_name()?.to_str()?.to_string()));
            let ids = on_pci.and_then(|p| hex(&p.join("vendor")).zip(hex(&p.join("device"))));
            let name = read_trim(&dev.join("product_name"))
                .filter(|s| !s.is_empty())
                .or_else(|| ids.and_then(|(v, d)| names.name(root, v, d)))
                .unwrap_or_else(|| format!("{driver} GPU"));
            Some((
                n,
                Card {
                    card,
                    dev,
                    pci,
                    driver,
                    name,
                },
            ))
        })
        .collect();
    cards.sort_by_key(|c| c.0);
    cards.into_iter().map(|(_, c)| c).collect()
}

/// AMD's busy share of the whole device, 0 to 1.
pub fn amd_busy(dev: &Path) -> Option<f64> {
    let pct: f64 = read_trim(&dev.join("gpu_busy_percent"))?.parse().ok()?;
    Some((pct / 100.0).clamp(0.0, 1.0))
}

/// AMD's video memory in use and in all, in bytes.
pub fn amd_vram(dev: &Path) -> Option<(u64, u64)> {
    let n = |f: &str| read_trim(&dev.join(f))?.parse::<u64>().ok();
    Some((n("mem_info_vram_used")?, n("mem_info_vram_total")?)).filter(|&(_, t)| t > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDS: &str = "# pci.ids\n1002  Advanced Micro Devices, Inc. [AMD/ATI]\n\
        \t1636  Renoir\n\t1638  Cezanne [Radeon Vega Series / Radeon Vega Mobile Series]\n\
        \t\t17aa 5081  ThinkPad\n1af4  Red Hat, Inc.\n\t1050  Virtio 1.0 GPU\n\
        8086  Intel Corporation\n\t46d4  Alder Lake-N [UHD Graphics]\n";

    /// A card on device `path` (`0000:07:00.0`, or `0000:00:01.0/virtio0` for a device on a PCI
    /// transport) bound to `driver`, the PCI device holding `ids`.
    fn card(root: &Path, n: u32, path: &str, driver: &str, ids: Option<(&str, &str)>) -> PathBuf {
        let dev = root.join("sys/devices/pci0000:00").join(path);
        std::fs::create_dir_all(&dev).unwrap();
        let pci = if path.contains("virtio") {
            dev.parent().unwrap().to_path_buf()
        } else {
            dev.clone()
        };
        let bus = root.join("sys/bus/pci");
        std::fs::create_dir_all(&bus).unwrap();
        if ids.is_some() {
            std::os::unix::fs::symlink(&bus, pci.join("subsystem")).unwrap();
        }
        let drv = root.join("sys/bus/pci/drivers").join(driver);
        std::fs::create_dir_all(&drv).unwrap();
        std::os::unix::fs::symlink(&drv, dev.join("driver")).unwrap();
        if let Some((v, d)) = ids {
            std::fs::write(pci.join("vendor"), format!("{v}\n")).unwrap();
            std::fs::write(pci.join("device"), format!("{d}\n")).unwrap();
        }
        let c = root.join(format!("sys/class/drm/card{n}"));
        std::fs::create_dir_all(&c).unwrap();
        std::os::unix::fs::symlink(&dev, c.join("device")).unwrap();
        std::fs::create_dir_all(root.join(format!("sys/class/drm/card{n}-DP-1"))).unwrap();
        dev
    }

    #[test]
    fn cards_are_found_named_and_amd_reads_its_own_counters() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        std::fs::create_dir_all(r.join("usr/share/hwdata")).unwrap();
        std::fs::write(r.join("usr/share/hwdata/pci.ids"), IDS).unwrap();
        let amd = card(r, 1, "0000:07:00.0", "amdgpu", Some(("0x1002", "0x1638")));
        card(r, 0, "simple-framebuffer.0", "simple-framebuffer", None);
        card(
            r,
            2,
            "0000:00:01.0/virtio0",
            "virtio_gpu",
            Some(("0x1af4", "0x1050")),
        );
        std::fs::write(amd.join("gpu_busy_percent"), "37\n").unwrap();
        std::fs::write(amd.join("mem_info_vram_used"), "268435456\n").unwrap();
        std::fs::write(amd.join("mem_info_vram_total"), "536870912\n").unwrap();
        let cards = scan(r, &mut Names::default());
        let got: Vec<(&str, Option<&str>, &str)> = (cards.iter())
            .map(|c| (c.card.as_str(), c.pci.as_deref(), c.driver.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("card1", Some("0000:07:00.0"), "amdgpu"),
                ("card2", Some("0000:00:01.0"), "virtio_gpu")
            ]
        );
        assert_eq!(
            cards[0].name,
            "AMD Radeon Vega Series / Radeon Vega Mobile Series"
        );
        assert_eq!(
            cards[1].name, "Red Hat Virtio 1.0 GPU",
            "named by the PCI device it sits on"
        );
        assert_eq!(amd_busy(&cards[0].dev), Some(0.37));
        assert_eq!(amd_vram(&cards[0].dev), Some((256 << 20, 512 << 20)));
        assert_eq!(amd_busy(&cards[1].dev), None);
    }
}
