//! `Place`: everywhere Explorer's address bar and tree can point. `Folder` is any
//! real directory; the others are the shell's special places, of which `Computer`
//! has no backing directory.

use reprisefs::listing::Item;
use reprisefs::places::{Drive, DriveKind, home_label};
use reprisefs::udisks::{Disk, Snapshot, Volume};
use reprisekit::controls::shellview::Field;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    Folder(PathBuf),
    Desktop,
    Documents,
    Home,
    Computer,
    RecycleBin,
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// The Desktop folder, `XDG_DESKTOP_DIR` else `~/Desktop`, read once per process
/// (`from_path` runs for every folder a tree node lists).
fn desktop() -> Option<PathBuf> {
    static DESKTOP: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    DESKTOP
        .get_or_init(|| reprisefs::places::user_dir("DESKTOP", "Desktop"))
        .clone()
}

/// The Documents folder, `XDG_DOCUMENTS_DIR` else `~/Documents`, read once.
fn documents() -> Option<PathBuf> {
    static DOCUMENTS: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    DOCUMENTS
        .get_or_init(|| reprisefs::places::user_dir("DOCUMENTS", "Documents"))
        .clone()
}

/// The real folders that stand for a shell place.
pub struct Specials<'a> {
    pub home: &'a Path,
    pub documents: Option<&'a Path>,
    pub desktop: Option<&'a Path>,
    pub trash: Option<&'a Path>,
}

impl Place {
    /// `p` compared exactly (nothing canonicalised) against `$HOME`, the Documents
    /// and Desktop folders and the trash's `files/`.
    pub fn from_path(p: &Path) -> Place {
        match home() {
            Some(h) => Self::from_path_in(
                p,
                &Specials {
                    home: &h,
                    documents: documents().as_deref(),
                    desktop: desktop().as_deref(),
                    trash: reprisefs::places::trash_dir().as_deref(),
                },
            ),
            None => Place::Folder(p.to_path_buf()),
        }
    }

    /// `from_path` against explicit folders, so a test need not touch `$HOME`. A
    /// Documents folder set to `$HOME` itself is Home.
    pub fn from_path_in(p: &Path, at: &Specials) -> Place {
        if p == at.home {
            Place::Home
        } else if at.documents == Some(p) {
            Place::Documents
        } else if at.desktop == Some(p) {
            Place::Desktop
        } else if at.trash == Some(p) {
            Place::RecycleBin
        } else {
            Place::Folder(p.to_path_buf())
        }
    }

    pub fn path(&self) -> Option<PathBuf> {
        match self {
            Place::Folder(p) => Some(p.clone()),
            Place::Desktop => desktop(),
            Place::Documents => documents(),
            Place::Home => home(),
            Place::RecycleBin => reprisefs::places::trash_dir(),
            Place::Computer => None,
        }
    }

    pub fn title(&self) -> String {
        match self {
            Place::Folder(p) => match p.file_name() {
                Some(name) => name.to_string_lossy().into_owned(),
                None => "/".to_string(),
            },
            Place::Desktop => "Desktop".to_string(),
            Place::Documents => "Documents".to_string(),
            Place::Home => home_label(),
            Place::Computer => "Computer".to_string(),
            Place::RecycleBin => "Recycle Bin".to_string(),
        }
    }

    pub fn address(&self) -> String {
        match self {
            Place::Folder(p) => p.display().to_string(),
            _ => self.title(),
        }
    }

    pub fn icon_name(&self) -> &'static str {
        match self {
            Place::Folder(_) => "folder",
            Place::Desktop => "user-desktop",
            Place::Documents => "folder-documents",
            Place::Home => "user-home",
            Place::Computer => "computer",
            Place::RecycleBin => "user-trash",
        }
    }

    pub fn shell(&self) -> Option<reprisefs::places::ShellPlace> {
        use reprisefs::places::ShellPlace;
        match self {
            Place::Documents => Some(ShellPlace::Documents),
            Place::Computer => Some(ShellPlace::Computer),
            Place::Home => Some(ShellPlace::Home),
            Place::RecycleBin => Some(ShellPlace::RecycleBin),
            Place::Folder(_) | Place::Desktop => None,
        }
    }

    /// Where Up goes, given the mount points Computer lists. A drive's root (and
    /// `/`) goes up to Computer, not the directory it is mounted in. Shell places go
    /// up to the Desktop.
    pub fn parent_in(&self, mounts: &[PathBuf]) -> Option<Place> {
        match self {
            Place::Folder(p) if p == Path::new("/") || mounts.iter().any(|m| m == p) => {
                Some(Place::Computer)
            }
            Place::Folder(p) => p.parent().map(Place::from_path),
            Place::Documents | Place::Home | Place::Computer | Place::RecycleBin => {
                Some(Place::Desktop)
            }
            Place::Desktop => None,
        }
    }

    pub fn parent(&self) -> Option<Place> {
        self.parent_in(&[])
    }

    pub fn read_only(&self) -> bool {
        matches!(self, Place::Computer | Place::RecycleBin)
    }

    /// Whether the Details pane calls this a "System Folder": every shell place,
    /// including the three that are real writable directories.
    pub fn is_system(&self) -> bool {
        !matches!(self, Place::Folder(_))
    }

    /// Typed address-bar text or a command-line place: "Computer" and "Recycle Bin"
    /// (case-insensitive), the text the address bar shows for `here`, `~`/`~/...`, or
    /// an absolute existing directory.
    pub fn parse_address(s: &str, here: Option<&Place>) -> Option<Place> {
        let (place, dir) = Self::read_address(s, here)?;
        dir.is_none_or(|d| d.is_dir()).then_some(place)
    }

    /// [`Place::parse_address`] without touching the file system: the place, and the folder
    /// still to be checked to exist when the text is a path.
    pub fn read_address(s: &str, here: Option<&Place>) -> Option<(Place, Option<PathBuf>)> {
        let s = s.trim();
        let lower = s.to_lowercase();
        for (name, place) in [
            ("computer", Place::Computer),
            ("recycle bin", Place::RecycleBin),
        ] {
            if lower == name {
                return Some((place, None));
            }
        }
        if let Some(here) = here.filter(|p| p.address() == s) {
            return Some((here.clone(), None));
        }
        let expanded = if s == "~" {
            home()?
        } else if let Some(rest) = s.strip_prefix("~/") {
            home()?.join(rest)
        } else {
            PathBuf::from(s)
        };
        expanded
            .is_absolute()
            .then(|| (Place::from_path(&expanded), Some(expanded)))
    }

    /// The places from the Desktop down to `self`: the path the Folders tree walks to
    /// reach it, given the mount points Computer lists. Each link is
    /// [`Place::parent_in`], so the walk stops at the first mount root above the
    /// place and hangs it off Computer. With `/` and `/home` mounted, `/home/u/x`
    /// comes out as Desktop, Computer, `/home`, `/home/u`, `/home/u/x`.
    ///
    /// Capped at 64 links against a pathological parent chain.
    pub fn chain_in(&self, mounts: &[PathBuf]) -> Vec<Place> {
        let mut chain = vec![self.clone()];
        while let Some(p) = chain.last().and_then(|c| c.parent_in(mounts)) {
            chain.push(p);
            if chain.len() > 64 {
                break;
            }
        }
        chain.reverse();
        chain
    }

    pub fn chain(&self) -> Vec<Place> {
        self.chain_in(&[])
    }
}

pub fn not_found_text(typed: &str) -> String {
    format!("Cannot find '{typed}'. Check the spelling and try again.")
}

/// A folder listing, or (for `Computer`) the three device groups: name and the
/// index into `items` where it starts. Empty groups are left out.
pub struct Listed {
    pub items: Vec<Item>,
    pub groups: Vec<(String, usize)>,
    /// The drives behind Computer's rows, for Details, the status bar and the tree.
    pub drives: Vec<Drive>,
}

/// Mount points of the drives Computer lists. A subvolume on the root's device
/// is not a drive.
pub fn drive_mounts(drives: &[Drive]) -> Vec<PathBuf> {
    drives.iter().map(|d| d.mount.clone()).collect()
}

/// A tree node's child folders: `listed`'s directories minus any that is a
/// drive's mount point; a drive shows once, under Computer.
pub fn tree_subfolders(listed: &Listed, mounts: &[PathBuf]) -> Vec<(String, Place)> {
    listed
        .items
        .iter()
        .filter(|it| it.is_folder() && !mounts.iter().any(|m| m == &it.path))
        .map(|it| (it.name.clone(), Place::from_path(&it.path)))
        .collect()
}

/// Computer's children in the Folders tree: the mounted drives, then Home. An
/// unmounted volume has no folder yet and is left out.
pub fn computer_tree_children(listed: &Listed) -> Vec<(String, Place)> {
    let mut kids: Vec<(String, Place)> = listed
        .drives
        .iter()
        .map(|d| (d.name(), Place::Folder(d.mount.clone())))
        .collect();
    // The "Files Stored on This Computer" row: the one item that is not a drive.
    for it in listed.items.iter().filter(|it| it.drive.is_none()) {
        kids.push((it.name.clone(), Place::from_path(&it.path)));
    }
    kids
}

/// The UDisks2 volume behind Computer's drive row `it`: mounted at its path, or
/// unmounted with its device as the path.
pub fn row_volume<'a>(volumes: &'a Snapshot, it: &Item) -> Option<&'a Volume> {
    it.drive?;
    volumes.volumes.iter().find(|v| {
        v.mounts.contains(&it.path) || (v.mounts.is_empty() && Path::new(&v.device) == it.path)
    })
}

pub fn unmounted_volume<'a>(volumes: &'a Snapshot, it: &Item) -> Option<&'a Volume> {
    row_volume(volumes, it).filter(|v| v.mounts.is_empty())
}

/// What Eject on row `it` removes: its volume and the removable drive holding it.
/// `None` for a fixed disk or a drive UDisks2 does not know.
pub fn eject_target<'a>(volumes: &'a Snapshot, it: &Item) -> Option<(&'a Volume, &'a Disk)> {
    let v = row_volume(volumes, it).filter(|v| volumes.removable(v))?;
    Some((v, volumes.disk_of(v)?))
}

fn empty_listed() -> Listed {
    Listed {
        items: Vec::new(),
        groups: Vec::new(),
        drives: Vec::new(),
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum ColumnKey {
    Name,
    Folder(PathBuf),
    Size(u64),
    Kind(String),
    Date(Option<i64>),
}

/// The folder an In Folder or Original Location column names: where an item in
/// the Recycle Bin was deleted from, else where it is.
pub fn folder_of(it: &Item) -> Option<&Path> {
    match &it.trashed {
        Some(t) => t.original.parent(),
        None => it.path.parent(),
    }
}

/// `items`' indices in the order a column showing `field` puts them. Ascending
/// lists folders ([`Item::is_folder`]) first, each part by the column's key (name
/// and kind case-insensitively, size and dates numerically, folder by path), then
/// by name; descending reverses that. Keys are built once per item.
pub fn sort_order(items: &[Item], field: Field, asc: bool) -> Vec<usize> {
    let key = |i: usize| {
        let it = &items[i];
        let by = match field {
            Field::Folder => {
                ColumnKey::Folder(folder_of(it).map(Path::to_path_buf).unwrap_or_default())
            }
            Field::Size => ColumnKey::Size(it.size),
            Field::Kind => ColumnKey::Kind(it.kind.to_lowercase()),
            Field::Modified => ColumnKey::Date(it.modified),
            Field::Deleted => ColumnKey::Date(it.trashed.as_ref().and_then(|t| t.deleted)),
            Field::Name => ColumnKey::Name,
        };
        (by, it.label().to_lowercase())
    };
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_cached_key(|&i| (!items[i].is_folder(), key(i)));
    if !asc {
        order.reverse();
    }
    order
}

/// [`sort_order`] run on each group's span, `firsts` being the groups' first
/// indices. An ungrouped listing (`firsts` empty) sorts whole.
pub fn sort_order_grouped(items: &[Item], firsts: &[usize], field: Field, asc: bool) -> Vec<usize> {
    let mut bounds: Vec<usize> = Vec::with_capacity(firsts.len() + 2);
    if firsts.first() != Some(&0) {
        bounds.push(0);
    }
    bounds.extend_from_slice(firsts);
    bounds.push(items.len());
    bounds
        .windows(2)
        .flat_map(|w| {
            sort_order(&items[w[0]..w[1]], field, asc)
                .into_iter()
                .map(move |i| i + w[0])
        })
        .collect()
}

pub fn bin_dirs() -> Vec<PathBuf> {
    reprisefs::trash::Bins::system()
        .map(|b| b.files_dirs())
        .unwrap_or_default()
}

/// What the Recycle Bin view watches: the home trash's folders
/// (`places::trash_watches`), so a trash made or moved back while it is shown
/// lists, and each other drive's files folder.
pub fn bin_watches() -> Vec<(PathBuf, Option<OsString>)> {
    let Some(bins) = reprisefs::trash::Bins::system() else {
        return Vec::new();
    };
    let roots = bins.roots();
    let Some((home, drives)) = roots.split_first() else {
        return Vec::new();
    };
    let drives = drives
        .iter()
        .map(|r| r.join("files"))
        .filter(|f| f.is_dir())
        .map(|f| (f, None));
    reprisefs::places::trash_watches(home)
        .into_iter()
        .chain(drives)
        .collect()
}

/// `place`'s contents: a real directory read (for `RecycleBin` each of
/// [`bin_dirs`]) filtered by `hidden` unless `show_hidden`; `Computer` builds the
/// three groups from `drives` and the unmounted ones in `volumes`.
pub fn list(
    place: &Place,
    globs: &reprisefs::apps::MimeDb,
    show_hidden: bool,
    drives: &[Drive],
    volumes: &Snapshot,
) -> Listed {
    match place {
        Place::Computer => list_computer(drives, volumes),
        Place::RecycleBin => {
            let mut items: Vec<Item> = bin_dirs()
                .iter()
                .flat_map(|dir| reprisefs::listing::read_dir(dir, globs))
                .filter(|i| show_hidden || !i.hidden)
                .collect();
            for it in &mut items {
                it.trashed = reprisefs::trash::trashed(&it.path).ok().map(Box::new);
            }
            Listed {
                items,
                groups: Vec::new(),
                drives: Vec::new(),
            }
        }
        _ => {
            let Some(dir) = place.path() else {
                return empty_listed();
            };
            let mut items = reprisefs::listing::read_dir(&dir, globs);
            if !show_hidden {
                items.retain(|i| !i.hidden);
            }
            Listed {
                items,
                groups: Vec::new(),
                drives: Vec::new(),
            }
        }
    }
}

fn drive_item(d: &Drive) -> Item {
    Item {
        name: d.name(),
        path: d.mount.clone(),
        is_dir: true,
        is_link: false,
        exec: false,
        program: false,
        hidden: false,
        size: d.total,
        modified: None,
        mime: None,
        kind: d.kind().label().to_string(),
        drive: Some(d.kind()),
        mount_root: false,
        trashed: None,
        shortcut: None,
    }
}

/// An unmounted removable volume's row: named and iconed like the drive it
/// becomes once mounted, with its device where the mount point would be.
fn volume_item(volumes: &Snapshot, v: &Volume) -> Item {
    let disk = volumes.disk_of(v);
    let kind = if disk.is_some_and(|d| d.optical) {
        DriveKind::Optical
    } else {
        DriveKind::Removable
    };
    Item {
        name: format!("{} ({})", v.name(disk), v.device),
        path: PathBuf::from(&v.device),
        is_dir: true,
        is_link: false,
        exec: false,
        program: false,
        hidden: false,
        size: v.size,
        modified: None,
        mime: None,
        kind: kind.label().to_string(),
        drive: Some(kind),
        mount_root: false,
        trashed: None,
        shortcut: None,
    }
}

fn list_computer(drives: &[Drive], volumes: &Snapshot) -> Listed {
    build_computer(drives.to_vec(), volumes, home().map(|h| (h, home_label())))
}

/// Computer: "Files Stored on This Computer" (the home folder `home`),
/// "Hard Disk Drives" (non-removable, non-optical), then "Devices with Removable
/// Storage" (removable or optical drives, then unmounted removable `volumes`),
/// each omitted when empty. Split from `list_computer` so a test can drive it
/// with fake drives.
fn build_computer(
    drives: Vec<Drive>,
    volumes: &Snapshot,
    home: Option<(PathBuf, String)>,
) -> Listed {
    let mut items = Vec::new();
    let mut groups = Vec::new();

    if let Some((home, label)) = home {
        groups.push(("Files Stored on This Computer".to_string(), items.len()));
        items.push(Item {
            name: label,
            path: home,
            is_dir: true,
            is_link: false,
            exec: false,
            program: false,
            hidden: false,
            size: 0,
            modified: None,
            mime: None,
            kind: "File Folder".to_string(),
            drive: None,
            mount_root: false,
            trashed: None,
            shortcut: None,
        });
    }

    let (hard, removable): (Vec<&Drive>, Vec<&Drive>) =
        drives.iter().partition(|d| d.kind() == DriveKind::Local);
    if !hard.is_empty() {
        groups.push(("Hard Disk Drives".to_string(), items.len()));
        items.extend(hard.into_iter().map(drive_item));
    }
    // A volume `drives` already lists as mounted stays out even when
    // `volumes` was read before the mount.
    let unmounted: Vec<Item> = volumes
        .unmounted()
        .into_iter()
        .filter(|v| !drives.iter().any(|d| d.device == v.device))
        .map(|v| volume_item(volumes, v))
        .collect();
    if !removable.is_empty() || !unmounted.is_empty() {
        groups.push(("Devices with Removable Storage".to_string(), items.len()));
        items.extend(removable.into_iter().map(drive_item));
        items.extend(unmounted);
    }

    Listed {
        items,
        groups,
        drives,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        PathBuf::from("/home/u")
    }

    #[test]
    fn from_path_recognizes_home_the_xdg_folders_and_the_trash() {
        let docs = home().join("Dokumente");
        let desk = home().join("Schreibtisch");
        let trash = home().join(".local/share/Trash/files");
        let h = home();
        let at = Specials {
            home: &h,
            documents: Some(&docs),
            desktop: Some(&desk),
            trash: Some(&trash),
        };
        let from = |p: &Path| Place::from_path_in(p, &at);
        assert_eq!(from(&home()), Place::Home);
        assert_eq!(from(&docs), Place::Documents);
        assert_eq!(
            from(&home().join("Documents")),
            Place::Folder(home().join("Documents")),
            "~/Documents is a plain folder when XDG_DOCUMENTS_DIR is elsewhere"
        );
        assert_eq!(from(&desk), Place::Desktop);
        assert_eq!(
            from(&home().join("Desktop")),
            Place::Folder(home().join("Desktop")),
            "~/Desktop is a plain folder when XDG_DESKTOP_DIR is elsewhere"
        );
        assert_eq!(from(&home().join("x")), Place::Folder(home().join("x")));
        assert_eq!(
            from(&trash),
            Place::RecycleBin,
            "the trash's files folder is the Recycle Bin, which takes no writes"
        );
        // Documents set to the home folder itself is Home.
        let same = Specials {
            documents: Some(&h),
            ..at
        };
        assert_eq!(Place::from_path_in(&h, &same), Place::Home);
    }

    /// The shell places' names, as the title bar and the address bar
    /// show them.
    #[test]
    fn the_shell_places_are_titled_computer_documents_and_the_users_home() {
        assert_eq!(Place::Computer.title(), "Computer");
        assert_eq!(Place::Documents.title(), "Documents");
        assert_eq!(Place::Home.title(), home_label());
        assert_eq!(Place::Documents.address(), "Documents");
        assert_eq!(Place::Home.icon_name(), "user-home");
        // A listing shows the home folder with Home's icon too.
        assert_eq!(
            reprisefs::places::special_folders().first(),
            super::home().map(|h| (h, "user-home")).as_ref()
        );
    }

    #[test]
    fn parent_chain_reaches_computer_then_desktop_then_none() {
        assert_eq!(
            Place::Folder(PathBuf::from("/")).parent(),
            Some(Place::Computer)
        );
        assert_eq!(Place::Computer.parent(), Some(Place::Desktop));
        assert_eq!(Place::Desktop.parent(), None);
        assert_eq!(Place::Documents.parent(), Some(Place::Desktop));
        assert_eq!(Place::Home.parent(), Some(Place::Desktop));
        assert_eq!(Place::RecycleBin.parent(), Some(Place::Desktop));
    }

    /// Only Computer and Recycle Bin are names; the old long names and
    /// Documents are not, outside the window already showing it.
    #[test]
    fn parse_address_matches_special_names_case_insensitively() {
        assert_eq!(
            Place::parse_address("Computer", None),
            Some(Place::Computer)
        );
        assert_eq!(
            Place::parse_address("COMPUTER", None),
            Some(Place::Computer)
        );
        assert_eq!(
            Place::parse_address("recycle bin", None),
            Some(Place::RecycleBin)
        );
        for gone in [
            "My Computer",
            "My Documents",
            "My Network Places",
            "Documents",
        ] {
            assert_eq!(Place::parse_address(gone, None), None, "{gone}");
        }
        // Enter on the address the window shows re-lists that place.
        let docs = Some(&Place::Documents);
        assert_eq!(
            Place::parse_address("Documents", docs),
            Some(Place::Documents)
        );
        let home = Place::Home;
        assert_eq!(
            Place::parse_address(&home_label(), Some(&home)),
            Some(Place::Home)
        );
    }

    #[test]
    fn parse_address_expands_home_and_rejects_missing_paths() {
        assert_eq!(Place::parse_address("~", None), Some(Place::Home));
        assert_eq!(Place::parse_address("/no/such/dir", None), None);
        assert_eq!(
            Place::parse_address("/tmp", None),
            Some(Place::from_path(Path::new("/tmp")))
        );
    }

    /// What the address bar puts in the box when the text is nowhere.
    #[test]
    fn an_unparsable_address_gets_not_found_wording() {
        assert_eq!(Place::parse_address("nowhere", None), None);
        assert_eq!(
            not_found_text("nowhere"),
            "Cannot find 'nowhere'. Check the spelling and try again."
        );
    }

    /// `/usr/share` hangs off the root drive under Computer, which is
    /// the chain `tree_follow` expands, and the root drive is a child
    /// Computer really offers.
    #[test]
    fn a_plain_folder_hangs_off_the_root_drive_under_computer() {
        assert_eq!(
            Place::Folder(PathBuf::from("/usr/share")).chain(),
            vec![
                Place::Desktop,
                Place::Computer,
                Place::Folder(PathBuf::from("/")),
                Place::Folder(PathBuf::from("/usr")),
                Place::Folder(PathBuf::from("/usr/share")),
            ]
        );
        let mounts = "/dev/sda2 / ext4 rw 0 0\n/dev/sr0 /run/media/u/DVD iso9660 ro 0 0\n";
        let labels = |_: &str| None;
        let drives = reprisefs::places::drives_from(mounts, &|_| false, &labels, &|_| (1000, 500));
        let listed = build_computer(
            drives,
            &Snapshot::default(),
            Some((PathBuf::from("/home/u"), "u's Home".to_string())),
        );
        let kids = computer_tree_children(&listed);
        // The drives first, named as the view names them, then the
        // user's Home.
        assert_eq!(
            kids,
            vec![
                (
                    "Local Disk (/)".to_string(),
                    Place::Folder(PathBuf::from("/"))
                ),
                (
                    "CD Drive (/run/media/u/DVD)".to_string(),
                    Place::Folder(PathBuf::from("/run/media/u/DVD"))
                ),
                (
                    "u's Home".to_string(),
                    Place::Folder(PathBuf::from("/home/u"))
                ),
            ]
        );
    }

    /// A machine with `/home` on its own device: everything under it
    /// hangs off that drive, `/`'s tree listing does not repeat it, and
    /// its root's Up is Computer.
    #[test]
    fn a_second_mount_owns_its_subtree_and_shows_once() {
        let mounts = vec![PathBuf::from("/"), PathBuf::from("/home")];
        assert_eq!(
            Place::Folder(PathBuf::from("/home/u/x")).chain_in(&mounts),
            vec![
                Place::Desktop,
                Place::Computer,
                Place::Folder(PathBuf::from("/home")),
                Place::Folder(PathBuf::from("/home/u")),
                Place::Folder(PathBuf::from("/home/u/x")),
            ]
        );
        assert_eq!(
            Place::Folder(PathBuf::from("/home")).parent_in(&mounts),
            Some(Place::Computer)
        );
        // Without that mount the same path walks `/` instead.
        assert_eq!(
            Place::Folder(PathBuf::from("/home")).parent_in(&[PathBuf::from("/")]),
            Some(Place::Folder(PathBuf::from("/")))
        );

        // `/`'s own tree children: `home` is the other drive's root and
        // belongs under Computer, `usr` is a plain folder.
        let dir = |name: &str| Item {
            name: name.to_string(),
            path: PathBuf::from("/").join(name),
            is_dir: true,
            is_link: false,
            exec: false,
            program: false,
            hidden: false,
            size: 0,
            modified: None,
            mime: None,
            kind: "File Folder".to_string(),
            drive: None,
            mount_root: false,
            trashed: None,
            shortcut: None,
        };
        // A symlink to a folder is a shortcut, which the tree leaves out.
        let link = Item {
            is_link: true,
            kind: "Shortcut".to_string(),
            ..dir("usr link")
        };
        let listed = Listed {
            items: vec![dir("home"), dir("usr"), link],
            groups: Vec::new(),
            drives: Vec::new(),
        };
        assert_eq!(
            tree_subfolders(&listed, &mounts),
            vec![("usr".to_string(), Place::Folder(PathBuf::from("/usr")))]
        );
        // With no second mount both are plain folders.
        assert_eq!(tree_subfolders(&listed, &[PathBuf::from("/")]).len(), 2);
    }

    #[test]
    fn read_only_matches_the_places_that_take_no_writes() {
        assert!(Place::Computer.read_only());
        assert!(Place::RecycleBin.read_only());
        // Home and Documents are real folders that take writes.
        assert!(!Place::Home.read_only());
        assert!(!Place::Documents.read_only());
        assert!(!Place::Desktop.read_only());
        assert!(!Place::Folder(PathBuf::from("/tmp")).read_only());
    }

    #[test]
    fn build_computer_orders_documents_then_hard_disks_then_removable() {
        let mounts = "/dev/sda2 / ext4 rw 0 0\n\
             /dev/sr0 /run/media/u/DVD iso9660 ro 0 0\n\
             tmpfs /tmp tmpfs rw 0 0\n";
        let labels = |d: &str| (d == "/dev/sr0").then(|| "MYDVD".to_string());
        let drives = reprisefs::places::drives_from(mounts, &|_| false, &labels, &|_| (1000, 500));
        let listed = build_computer(
            drives,
            &Snapshot::default(),
            Some((PathBuf::from("/home/u"), "u's Home".to_string())),
        );

        assert_eq!(
            listed.groups,
            vec![
                ("Files Stored on This Computer".to_string(), 0),
                ("Hard Disk Drives".to_string(), 1),
                ("Devices with Removable Storage".to_string(), 2),
            ]
        );
        assert_eq!(listed.items[0].name, "u's Home");
        assert_eq!(listed.items[1].name, "Local Disk (/)");
        assert_eq!(listed.items[1].kind, "Local Disk");
        assert_eq!(listed.items[2].name, "MYDVD (/run/media/u/DVD)");
        assert_eq!(listed.items[2].kind, "CD Drive");
        assert_eq!(listed.items[0].drive, None);
        assert_eq!(listed.items[2].drive, Some(DriveKind::Optical));
        assert_eq!(listed.drives.len(), 2);
    }

    /// A stick with an unmounted KINGSTON partition, an unmounted
    /// partition on the fixed disk, and `/dev/sdc1` mounted at
    /// `/run/media/u/CARD` on a card reader.
    fn udisks_snapshot() -> Snapshot {
        let disk = |path: &str, removable: bool| Disk {
            path: path.into(),
            vendor: "Kingston".into(),
            model: "DataTraveler".into(),
            removable,
            ..Disk::default()
        };
        let volume = |dev: &str, disk: &str, label: Option<&str>, mounts: &[&str]| Volume {
            path: format!("/v{dev}"),
            device: dev.into(),
            disk: Some(disk.into()),
            label: label.map(Into::into),
            filesystem: true,
            size: 64 << 20,
            mounts: mounts.iter().map(PathBuf::from).collect(),
            ..Volume::default()
        };
        Snapshot {
            disks: vec![
                disk("/stick", true),
                disk("/ssd", false),
                disk("/card", true),
            ],
            volumes: vec![
                volume("/dev/sda3", "/ssd", None, &[]),
                volume("/dev/sdb1", "/stick", Some("KINGSTON"), &[]),
                volume("/dev/sdc1", "/card", None, &["/run/media/u/CARD"]),
            ],
        }
    }

    #[test]
    fn build_computer_lists_unmounted_removable_volumes_with_the_removable_drives() {
        let mounts = "/dev/sda2 / ext4 rw 0 0\n/dev/sdc1 /run/media/u/CARD vfat rw 0 0\n";
        let removable = |d: &str| d == "sdc1";
        let drives =
            reprisefs::places::drives_from(mounts, &removable, &|_| None, &|_| (1000, 500));
        let listed = build_computer(drives, &udisks_snapshot(), None);

        assert_eq!(
            listed.groups,
            vec![
                ("Hard Disk Drives".to_string(), 0),
                ("Devices with Removable Storage".to_string(), 1),
            ]
        );
        let names: Vec<&str> = listed.items.iter().map(|it| it.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Local Disk (/)",
                "Removable Disk (/run/media/u/CARD)",
                "KINGSTON (/dev/sdb1)"
            ],
            "the unmounted KINGSTON follows the mounted card; the fixed disk's partition stays out"
        );
        let stick = &listed.items[2];
        assert_eq!(stick.path, PathBuf::from("/dev/sdb1"));
        assert_eq!(stick.drive, Some(DriveKind::Removable));
        assert_eq!(stick.kind, "Removable Disk");
        assert!(
            computer_tree_children(&listed)
                .iter()
                .all(|(name, _)| name != "KINGSTON (/dev/sdb1)"),
            "the Folders tree leaves the unmounted volume out"
        );
    }

    #[test]
    fn only_rows_on_a_removable_drive_have_an_eject_target() {
        let snap = udisks_snapshot();
        let row = |path: &str, drive: Option<DriveKind>| Item {
            name: String::new(),
            path: PathBuf::from(path),
            is_dir: true,
            is_link: false,
            exec: false,
            program: false,
            hidden: false,
            size: 0,
            modified: None,
            mime: None,
            kind: String::new(),
            drive,
            mount_root: false,
            trashed: None,
            shortcut: None,
        };
        let card = row("/run/media/u/CARD", Some(DriveKind::Removable));
        let stick = row("/dev/sdb1", Some(DriveKind::Removable));
        let fixed = row("/dev/sda3", Some(DriveKind::Removable));
        let folder = row("/run/media/u/CARD", None);

        assert_eq!(
            eject_target(&snap, &card).map(|(v, d)| (v.device.as_str(), d.path.as_str())),
            Some(("/dev/sdc1", "/card"))
        );
        assert_eq!(
            eject_target(&snap, &stick).map(|(v, _)| v.device.as_str()),
            Some("/dev/sdb1")
        );
        assert!(
            eject_target(&snap, &fixed).is_none(),
            "a fixed disk has no Eject"
        );
        assert!(
            eject_target(&snap, &folder).is_none(),
            "a plain folder has no Eject"
        );
        assert!(unmounted_volume(&snap, &stick).is_some());
        assert!(
            unmounted_volume(&snap, &card).is_none(),
            "the card is mounted"
        );
    }

    #[test]
    fn build_computer_omits_empty_groups() {
        let listed = build_computer(Vec::new(), &Snapshot::default(), None);
        assert!(listed.groups.is_empty());
        assert!(listed.items.is_empty());
    }
}
