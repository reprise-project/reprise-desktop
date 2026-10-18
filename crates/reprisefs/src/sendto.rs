//! The Send To submenu's rows, shared by the desktop's menu and Explorer's.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::{names, ops, places};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendTo {
    Zip,
    DesktopShortcut,
    Mail,
    Folder(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendToRow {
    pub label: String,
    pub icon: &'static str,
    pub dest: SendTo,
    pub enabled: bool,
}

/// The rows for a desktop folder that is there or not, the Documents folder and `drives`; this
/// touches no file, so a folder on a mount that stopped answering cannot hold it.
pub fn rows(desktop: bool, documents: Option<PathBuf>, drives: &[PathBuf]) -> Vec<SendToRow> {
    let row = |label: &str, icon: &'static str, dest: SendTo| SendToRow {
        label: label.to_string(),
        icon,
        dest,
        enabled: true,
    };
    let mut rows = vec![
        row("Compressed (zipped) Folder", "application-zip", SendTo::Zip),
        SendToRow {
            enabled: desktop,
            ..row(
                "Desktop (create shortcut)",
                "user-desktop",
                SendTo::DesktopShortcut,
            )
        },
        row("Mail Recipient", "internet-mail", SendTo::Mail),
    ];
    if let Some(docs) = documents {
        rows.push(row("Documents", "folder-documents", SendTo::Folder(docs)));
    }
    for d in drives {
        let label = d
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| d.display().to_string());
        rows.push(row(
            &label,
            "drive-removable-media",
            SendTo::Folder(d.clone()),
        ));
    }
    rows
}

/// The submenu's own folders as a read off the host's loop found them, since the desktop folder
/// can sit on a mount that never answers.
#[derive(Debug, Default)]
pub struct Dirs {
    desktop: bool,
    documents: Option<PathBuf>,
}

impl Dirs {
    /// Stats `desktop`, which waits for as long as its mount does not answer.
    pub fn read(desktop: &Path) -> Dirs {
        Dirs {
            desktop: desktop.is_dir(),
            documents: places::user_dir("DOCUMENTS", "Documents"),
        }
    }
}

/// The drives the submenu lists, as the last scan found them. The host lists the mount points
/// ([`places::MountedDrives::listed`]) off its loop, then stats each drive top
/// ([`places::usable_drive`]) on a thread of its own, since a drive can be a mount that never
/// answers: one that hangs keeps only its own stat.
#[derive(Debug)]
pub struct Drives {
    found: Vec<PathBuf>,
    /// None while a scan has it.
    scanner: Option<places::MountedDrives>,
    /// The mount table changed during a scan: scan again when it lands.
    again: bool,
    /// The mount points the last scan listed, in order.
    listed: Vec<PathBuf>,
    /// Each listed drive's last [`places::usable_drive`] answer.
    usable: HashMap<PathBuf, bool>,
    /// The drives whose stat is out. A drive that never answers keeps its stat, and no second
    /// one starts.
    stats: HashSet<PathBuf>,
}

impl Drives {
    pub fn new(scanner: places::MountedDrives) -> Drives {
        Drives {
            found: Vec::new(),
            scanner: Some(scanner),
            again: false,
            listed: Vec::new(),
            usable: HashMap::new(),
            stats: HashSet::new(),
        }
    }

    /// The listed drives whose last stat found them usable, in mount order.
    pub fn found(&self) -> &[PathBuf] {
        &self.found
    }

    /// The scanner for a scan, or None while one runs, which then scans again when it lands.
    pub fn start(&mut self) -> Option<places::MountedDrives> {
        let scanner = self.scanner.take();
        self.again = scanner.is_none();
        scanner
    }

    /// Takes a scan's list: whether another scan was asked for meanwhile, and the drives to stat
    /// (those listed with no stat out, now counted as out).
    pub fn land(
        &mut self,
        scanner: places::MountedDrives,
        listed: Vec<PathBuf>,
    ) -> (bool, Vec<PathBuf>) {
        self.scanner = Some(scanner);
        self.usable.retain(|t, _| listed.contains(t));
        let stat = (listed.iter())
            .filter(|t| self.stats.insert(t.to_path_buf()))
            .cloned()
            .collect();
        self.listed = listed;
        self.refind();
        (std::mem::take(&mut self.again), stat)
    }

    /// The listed drives to stat again (those with no stat out, now counted as out), so a drive
    /// that stops answering shows as such.
    pub fn restat(&mut self) -> Vec<PathBuf> {
        (self.listed.iter())
            .filter(|t| self.stats.insert(t.to_path_buf()))
            .cloned()
            .collect()
    }

    /// For a send to `dir`: None when it is no listed drive (send at once), Some(true) when a stat
    /// of it starts (send once it answers), Some(false) when its stat is out (send nothing).
    pub fn send_stat(&mut self, dir: &Path) -> Option<bool> {
        self.listed
            .iter()
            .any(|t| t == dir)
            .then(|| self.stats.insert(dir.to_path_buf()))
    }

    /// The Send To rows for `dirs` and the found drives, one whose stat is out greyed.
    pub fn send_to_rows(&self, dirs: &Dirs) -> Vec<SendToRow> {
        let mut rows = self::rows(dirs.desktop, dirs.documents.clone(), &self.found);
        for r in &mut rows {
            if let SendTo::Folder(d) = &r.dest {
                r.enabled &= !self.stats.contains(d);
            }
        }
        rows
    }

    /// Takes drive `top`'s stat: whether it changed `found`.
    pub fn land_stat(&mut self, top: PathBuf, usable: bool) -> bool {
        self.stats.remove(&top);
        if !self.listed.contains(&top) {
            return false;
        }
        self.usable.insert(top, usable);
        let before = std::mem::take(&mut self.found);
        self.refind();
        self.found != before
    }

    fn refind(&mut self) {
        self.found = (self.listed.iter())
            .filter(|t| self.usable.get(*t) == Some(&true))
            .cloned()
            .collect();
    }
}

pub fn mail_argv(paths: &[PathBuf]) -> Vec<OsString> {
    let mut argv = vec![OsString::from("xdg-email")];
    for p in paths {
        argv.push("--attach".into());
        argv.push(p.into());
    }
    argv
}

/// The zip row's command for `paths`, items of one folder whose other names are `existing`: one
/// archive named after the first item and numbered past an existing name. None for an empty
/// selection.
pub fn zip_argv(paths: &[PathBuf], existing: &[String]) -> Option<Vec<OsString>> {
    let zip = names::zip_path(paths.first()?, existing);
    Some(ops::zip_argv(paths, &zip))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_come_in_order_and_grey_a_missing_desktop() {
        let rows = rows(
            true,
            Some(PathBuf::from("/home/u/Documents")),
            &[PathBuf::from("/run/media/u/USB")],
        );
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Compressed (zipped) Folder",
                "Desktop (create shortcut)",
                "Mail Recipient",
                "Documents",
                "USB",
            ]
        );
        assert!(rows.iter().all(|r| r.enabled), "every row is live");
        assert_eq!(rows[1].dest, SendTo::DesktopShortcut);
        assert_eq!(rows[4].dest, SendTo::Folder("/run/media/u/USB".into()));
        assert_eq!(rows[4].icon, "drive-removable-media");

        let bare = super::rows(false, None, &[]);
        assert_eq!(bare.len(), 3);
        assert!(!bare[1].enabled);
    }

    #[test]
    fn zip_takes_the_whole_set_into_one_archive_named_after_the_first() {
        let paths: Vec<PathBuf> = vec!["/d/b2.txt".into(), "/d/a1.txt".into(), "/d/c3".into()];
        let argv = zip_argv(&paths, &["b2.zip".into()]).unwrap();
        assert_eq!(
            argv,
            [
                "bsdtar",
                "-a",
                "-cf",
                "/d/b2 (2).zip",
                "-C",
                "/d",
                "--",
                "b2.txt",
                "a1.txt",
                "c3"
            ]
        );
        assert_eq!(zip_argv(&[], &[]), None);
    }

    #[test]
    fn mail_attaches_every_path() {
        let argv = mail_argv(&["/a b".into(), "/c".into()]);
        assert_eq!(argv, ["xdg-email", "--attach", "/a b", "--attach", "/c"]);
    }

    #[test]
    fn one_drive_scan_runs_at_a_time_and_a_mount_change_during_it_scans_again() {
        let t = std::env::temp_dir().join(format!("reprise-drives-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let (root, table) = (t.join("media"), t.join("mounts"));
        std::fs::create_dir_all(root.join("STICK")).unwrap();
        let line = format!("/dev/sdb1 {} vfat rw 0 0\n", root.join("STICK").display());
        std::fs::write(&table, line).unwrap();
        let mut drives = Drives::new(places::MountedDrives::at(table, vec![root.clone()]));
        let mut scanner = drives.start().unwrap();
        assert!(drives.start().is_none(), "a scan is under way");
        let listed = scanner.listed().to_vec();
        let (again, stat) = drives.land(scanner, listed);
        assert!(again, "the table changed during the scan");
        assert_eq!(stat, [root.join("STICK")]);
        assert!(
            drives.found().is_empty(),
            "nothing listed before its stat lands"
        );
        drives.land_stat(root.join("STICK"), true);
        assert_eq!(drives.found(), [root.join("STICK")]);
        let scanner = drives.start().unwrap();
        assert_eq!(drives.land(scanner, Vec::new()), (false, Vec::new()));
        assert_eq!(drives.found(), Vec::<PathBuf>::new());
        let _ = std::fs::remove_dir_all(&t);
    }

    fn drives_listing(tops: &[PathBuf]) -> Drives {
        let mut drives = Drives::new(places::MountedDrives::at("/nonexistent".into(), Vec::new()));
        let scanner = drives.start().unwrap();
        drives.land(scanner, tops.to_vec());
        drives
    }

    #[test]
    fn a_drive_stat_landing_says_whether_the_found_drives_changed() {
        let (card, stick) = (PathBuf::from("/media/CARD"), PathBuf::from("/media/STICK"));
        let mut drives = drives_listing(&[card.clone(), stick.clone()]);
        assert!(drives.land_stat(card.clone(), true), "CARD answered");
        assert!(
            !drives.land_stat(stick.clone(), false),
            "STICK is not usable"
        );
        let scanner = drives.start().unwrap();
        let (_, stat) = drives.land(scanner, vec![card.clone(), stick.clone()]);
        assert_eq!(stat, [card.clone(), stick.clone()]);
        assert!(!drives.land_stat(card.clone(), true), "the same answer");
        assert!(drives.land_stat(stick.clone(), true), "STICK became usable");
        assert!(
            !drives.land_stat("/media/GONE".into(), true),
            "a drive not listed"
        );
    }

    #[test]
    fn a_found_drive_whose_new_stat_is_out_greys_its_send_to_row() {
        let (card, dead) = (PathBuf::from("/media/CARD"), PathBuf::from("/media/DEAD"));
        let mut drives = drives_listing(&[card.clone(), dead.clone()]);
        drives.land_stat(card.clone(), true);
        drives.land_stat(dead.clone(), true);
        let enabled = |d: &Drives, top: &PathBuf| {
            let rows = d.send_to_rows(&Dirs::default());
            let dest = SendTo::Folder(top.clone());
            let row = rows.iter().find(|r| r.dest == dest);
            row.map(|r| r.enabled)
        };
        assert_eq!(enabled(&drives, &dead), Some(true));

        assert_eq!(drives.restat(), [card.clone(), dead.clone()]);
        assert_eq!(drives.restat(), Vec::<PathBuf>::new(), "no second stat");
        drives.land_stat(card.clone(), true);
        assert_eq!(enabled(&drives, &card), Some(true));
        assert_eq!(enabled(&drives, &dead), Some(false), "DEAD's stat is out");
        drives.land_stat(dead.clone(), true);
        assert_eq!(enabled(&drives, &dead), Some(true), "DEAD answered");
    }

    #[test]
    fn send_to_rows_take_the_desktop_folder_as_the_desktop_read_found_it() {
        let drives = drives_listing(&[]);
        let desktop = |dirs: &Dirs| drives.send_to_rows(dirs)[1].enabled;
        assert!(!desktop(&Dirs::default()), "before the first read");
        let dir = std::env::temp_dir();
        assert!(desktop(&Dirs::read(&dir)));
        let gone = dir.join(format!("reprise-no-desktop-{}", std::process::id()));
        assert!(!desktop(&Dirs::read(&gone)));
    }

    #[test]
    fn a_send_to_a_drive_waits_on_a_fresh_stat_and_one_whose_stat_is_out_sends_nothing() {
        let dead = PathBuf::from("/media/DEAD");
        let mut drives = drives_listing(std::slice::from_ref(&dead));
        drives.land_stat(dead.clone(), true);
        assert_eq!(drives.send_stat(Path::new("/home/u/Documents")), None);
        assert_eq!(drives.send_stat(&dead), Some(true), "a fresh stat starts");
        assert_eq!(drives.send_stat(&dead), Some(false), "its stat is out");
        assert_eq!(
            drives.restat(),
            Vec::<PathBuf>::new(),
            "the send's stat counts"
        );
    }

    #[test]
    fn a_drive_whose_stat_never_lands_keeps_no_drive_mounted_later_off_the_list() {
        let (dead, card) = (PathBuf::from("/media/DEAD"), PathBuf::from("/media/CARD"));
        let mut drives = Drives::new(places::MountedDrives::at("/nonexistent".into(), Vec::new()));
        let scanner = drives.start().unwrap();
        let (_, stat) = drives.land(scanner, vec![dead.clone()]);
        assert_eq!(stat, std::slice::from_ref(&dead));

        let scanner = drives.start().unwrap();
        let (_, stat) = drives.land(scanner, vec![card.clone(), dead.clone()]);
        assert_eq!(
            stat,
            std::slice::from_ref(&card),
            "DEAD's stat is still out: no second one"
        );
        drives.land_stat(card.clone(), true);
        assert_eq!(drives.found(), std::slice::from_ref(&card));

        drives.land_stat(dead.clone(), true);
        assert_eq!(drives.found(), [card.clone(), dead.clone()], "mount order");
        let scanner = drives.start().unwrap();
        let (_, stat) = drives.land(scanner, vec![card.clone()]);
        assert_eq!(stat, std::slice::from_ref(&card));
        assert_eq!(
            drives.found(),
            std::slice::from_ref(&card),
            "DEAD unmounted"
        );
        drives.land_stat(dead, true);
        assert_eq!(
            drives.found(),
            [card],
            "a stat landing after its unmount lists nothing"
        );
    }
}
