//! The low disk space warning: the disks of `$HOME` and `/` are read every [`INTERVAL`] on a
//! thread of their own and warn once when one falls below its [`limit`], again only after it
//! has climbed a point of its size past the limit and fallen back.

use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const INTERVAL: Duration = Duration::from_secs(120);
/// Free space under this is low whatever the share; a tenth of a disk smaller than ten times it.
pub const FLOOR: u64 = 1 << 30;
/// The paths to watch instead of `$HOME` and `/`, `:`-separated: for a test on a small file
/// system of its own.
pub const PATHS_ENV: &str = "REPRISE_LOW_SPACE_PATHS";

/// One disk as last read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reading {
    pub dev: u64,
    pub mount: PathBuf,
    pub free: u64,
    pub total: u64,
    /// The Recycle Bin holds something.
    pub bin: bool,
}

/// The free bytes under which a disk of `total` bytes is low: `percent` of it, or [`FLOOR`]
/// when that is more (a tenth of the disk at most). None while `percent` is 0.
pub fn limit(total: u64, percent: u8) -> Option<u64> {
    (percent > 0).then(|| (total / 100 * u64::from(percent)).max(FLOOR.min(total / 10)))
}

/// The disks warned about, until each climbs back.
#[derive(Default)]
pub struct Warned(HashSet<u64>);

impl Warned {
    /// The readings to warn about now.
    pub fn feed<'a>(&mut self, readings: &'a [Reading], percent: u8) -> Vec<&'a Reading> {
        let mut warn = Vec::new();
        for r in readings {
            let Some(low) = limit(r.total, percent) else {
                self.0.clear();
                continue;
            };
            if r.free < low {
                if self.0.insert(r.dev) {
                    warn.push(r);
                }
            } else if r.free >= low + r.total / 100 {
                self.0.remove(&r.dev);
            }
        }
        warn
    }
}

/// The warning's text for `r`.
pub fn balloon(r: &Reading) -> (String, String) {
    (
        "Low Disk Space".into(),
        format!(
            "The disk of {} has only {} free of {}.",
            r.mount.display(),
            reprisefs::props::short_size(r.free),
            reprisefs::props::short_size(r.total),
        ),
    )
}

pub const OPEN: &str = "Open";
pub const SHOW_USAGE: &str = "Show Usage";
pub const EMPTY_BIN: &str = "Empty Recycle Bin";

/// The warning's buttons: Open, then Empty Recycle Bin while the bin holds something, else Show
/// Usage. Two keep both labels whole on the narrowest notification.
pub fn buttons(r: &Reading) -> [&'static str; 2] {
    [OPEN, if r.bin { EMPTY_BIN } else { SHOW_USAGE }]
}

fn watched() -> Vec<PathBuf> {
    match std::env::var_os(PATHS_ENV) {
        Some(v) => std::env::split_paths(&v).collect(),
        None => [
            std::env::var_os("HOME").map(PathBuf::from),
            Some("/".into()),
        ]
        .into_iter()
        .flatten()
        .collect(),
    }
}

/// The root of the file system `path` is on: the last ancestor on its device.
fn mount_of(path: &Path, dev: u64) -> PathBuf {
    let mut at = path;
    while let Some(up) = at.parent() {
        match std::fs::metadata(up) {
            Ok(m) if m.dev() == dev => at = up,
            _ => break,
        }
    }
    at.to_path_buf()
}

fn bin_has_items() -> bool {
    reprisefs::trash::Bins::system()
        .map(|b| b.files_dirs())
        .unwrap_or_default()
        .iter()
        .any(|f| !reprisefs::places::trash_empty(f))
}

/// Each watched disk once, in the order watched.
fn read() -> Vec<Reading> {
    let bin = bin_has_items();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for path in watched() {
        let Ok(dev) = std::fs::metadata(&path).map(|m| m.dev()) else {
            continue;
        };
        if !seen.insert(dev) {
            continue;
        }
        let Some((total, free)) = reprisefs::places::statvfs_totals(&path) else {
            continue;
        };
        out.push(Reading {
            dev,
            mount: mount_of(&path, dev),
            free,
            total,
            bin,
        });
    }
    out
}

/// Read the disks now and every [`INTERVAL`], posting each round.
pub fn watch(poster: repriseapp::Poster<Vec<Reading>>) {
    let run = move || {
        loop {
            poster.post(read());
            std::thread::sleep(INTERVAL);
        }
    };
    if let Err(e) = std::thread::Builder::new()
        .name("low-space".into())
        .spawn(run)
    {
        log::warn!("low disk space warning off: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    fn disk(dev: u64, free: u64, total: u64) -> Reading {
        Reading {
            dev,
            mount: "/".into(),
            free,
            total,
            bin: false,
        }
    }

    #[test]
    fn a_disk_is_low_below_its_share_or_the_floor() {
        assert_eq!(limit(2000 * GB, 5), Some(100 * GB));
        assert_eq!(limit(16 * GB, 5), Some(GB), "the floor on a small disk");
        assert_eq!(
            limit(64 << 20, 5),
            Some((64 << 20) / 10),
            "a tenth on a tiny one"
        );
        assert_eq!(limit(2000 * GB, 0), None, "0 turns it off");
    }

    #[test]
    fn a_disk_warns_once_per_fall_and_again_after_it_climbs_a_point() {
        let mut w = Warned::default();
        let at = |w: &mut Warned, free| w.feed(&[disk(1, free, 100 * GB)], 5).len();
        assert_eq!(at(&mut w, 6 * GB), 0);
        assert_eq!(at(&mut w, 4 * GB), 1, "it fell below 5 %");
        assert_eq!(at(&mut w, 3 * GB), 0, "still low: once");
        assert_eq!(at(&mut w, 5 * GB + GB / 2), 0, "above 5 % but under 6 %");
        assert_eq!(
            at(&mut w, 4 * GB),
            0,
            "not climbed far enough to warn again"
        );
        assert_eq!(at(&mut w, 6 * GB), 0, "6 % re-arms it");
        assert_eq!(at(&mut w, 4 * GB), 1, "the next fall warns");
    }

    #[test]
    fn each_disk_warns_for_itself_and_0_forgets_them() {
        let mut w = Warned::default();
        let both = [disk(1, GB, 100 * GB), disk(2, GB, 100 * GB)];
        assert_eq!(w.feed(&both, 5).len(), 2);
        assert_eq!(w.feed(&both, 0).len(), 0);
        assert_eq!(w.feed(&both, 5).len(), 2, "turned back on, both warn");
    }

    #[test]
    fn the_bin_button_shows_only_with_something_in_the_bin() {
        let mut r = disk(1, 812 << 20, 20 * GB);
        assert_eq!(buttons(&r), ["Open", "Show Usage"]);
        r.bin = true;
        assert_eq!(buttons(&r), ["Open", "Empty Recycle Bin"]);
        assert_eq!(
            balloon(&r),
            (
                "Low Disk Space".to_string(),
                "The disk of / has only 812 MB free of 20.0 GB.".to_string()
            )
        );
    }

    #[test]
    fn a_path_s_mount_is_its_highest_ancestor_on_the_same_device() {
        let dev = std::fs::metadata("/").unwrap().dev();
        assert_eq!(mount_of(Path::new("/"), dev), Path::new("/"));
        let proc_dev = std::fs::metadata("/proc/self").unwrap().dev();
        assert_eq!(
            mount_of(Path::new("/proc/self"), proc_dev),
            Path::new("/proc")
        );
    }

    #[test]
    fn both_button_sets_fit_whole_on_every_glass_sheet() {
        use reprisekit::controls::notifysheet::{SheetContent, sheet_layout};
        use reprisekit::style::SheetShape;
        let text = reprisekit::text::TextRenderer::load(&reprisekit::theme::FontConfig::default());
        let theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
        let mut r = disk(1, 812 << 20, 20 * GB);
        let (title, body) = balloon(&r);
        for bin in [false, true] {
            r.bin = bin;
            let actions = buttons(&r);
            let c = SheetContent {
                app: "Drives",
                when: "now",
                title: &title,
                body: &body,
                actions: &actions,
                action_icons: &[],
                reply: None,
                has_picture: false,
                has_app_icon: true,
            };
            for shape in [SheetShape::Lozenge, SheetShape::Halo] {
                let lay = sheet_layout(&theme, &text, shape, &c, false).unwrap();
                let whole: Vec<Option<String>> =
                    actions.iter().map(|a| Some(a.to_string())).collect();
                assert_eq!(lay.labels, whole, "{shape:?}, bin {bin}");
            }
        }
    }
}
