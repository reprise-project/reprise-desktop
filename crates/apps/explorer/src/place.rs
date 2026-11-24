//! `Place`: everywhere Explorer's address bar and tree can point. `Folder` is any
//! real directory; the others are the shell's special places, of which `Computer`
//! and `Network` have no backing directory.

use reprisefs::listing::Item;
use reprisefs::netshare::browse::{self, ROOT};
use reprisefs::netshare::{Address, Share};
use reprisefs::places::{Drive, DriveKind, home_label};
use reprisefs::portable::{self, Device};
use reprisefs::udisks::{Disk, Snapshot, Volume};
use reprisekit::controls::shellview::Field;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Place {
    Folder(PathBuf),
    Desktop,
    Documents,
    Home,
    Computer,
    RecycleBin,
    /// The servers on the network ([`reprisefs::netshare::browse::ROOT`]), or one SMB server's
    /// shares, by address.
    Network(String),
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

/// `p` as the address bar writes it: inside a mounted share or device, its name and the path
/// below its folder ("Test Phone/Internal shared storage"), else the path itself.
fn named_address(p: &Path, shares: &[Share]) -> String {
    let inside = (shares.iter()).find_map(|s| Some((&s.name, p.strip_prefix(&s.path).ok()?)));
    match inside {
        Some((name, rest)) if rest.as_os_str().is_empty() => name.clone(),
        Some((name, rest)) => format!("{name}/{}", rest.display()),
        None => p.display().to_string(),
    }
}

/// The folder typed text written as [`named_address`] writes it stands for.
fn named_path(text: &str, shares: &[Share]) -> Option<PathBuf> {
    (shares.iter()).find_map(|s| match text.strip_prefix(s.name.as_str())? {
        "" => Some(s.path.clone()),
        rest => Some(s.path.join(rest.strip_prefix('/')?)),
    })
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
            None => Place::of_row(p).unwrap_or_else(|| Place::Folder(p.to_path_buf())),
        }
    }

    /// `from_path` against explicit folders, so a test need not touch `$HOME`. A
    /// Documents folder set to `$HOME` itself is Home; a Desktop row's path is its place.
    pub fn from_path_in(p: &Path, at: &Specials) -> Place {
        if let Some(place) = Place::of_row(p) {
            place
        } else if p == at.home {
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
            Place::Computer | Place::Network(_) => None,
        }
    }

    /// The place's name; a mounted share's folder keeps gvfs's folder name ([`Place::title_in`]).
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
            Place::Network(uri) if uri == ROOT => "Network".to_string(),
            Place::Network(uri) => Address::parse(uri).map_or_else(|| uri.clone(), |a| a.host),
        }
    }

    /// The title, the folder of one of the mounted `shares` (phones and cameras among them) by its name.
    pub fn title_in(&self, shares: &[Share]) -> String {
        match self {
            Place::Folder(p) => (shares.iter().find(|s| s.path == *p))
                .map_or_else(|| self.title(), |s| s.name.clone()),
            _ => self.title(),
        }
    }

    /// The address bar's text, the mounted `shares` (phones and cameras among them) written by name.
    pub fn address_in(&self, shares: &[Share]) -> String {
        match self {
            Place::Folder(p) => named_address(p, shares),
            Place::Network(uri) if uri != ROOT => uri.clone(),
            _ => self.title(),
        }
    }

    /// The Network place itself.
    pub fn network() -> Place {
        Place::Network(ROOT.to_string())
    }

    /// The path of the Desktop view's row for one of [`desktop_places`]. Every system call refuses
    /// its leading NUL byte, so a file operation a row's guard misses touches nothing.
    pub fn row_path(&self) -> Option<PathBuf> {
        let name = match self {
            Place::Documents => "documents",
            Place::Home => "home",
            Place::Computer => "computer",
            Place::Network(uri) if uri == ROOT => "network",
            Place::RecycleBin => "recycle-bin",
            _ => return None,
        };
        Some(PathBuf::from(format!("\0{name}")))
    }

    /// The shell place a Desktop view row's `path` ([`Place::row_path`]) stands for.
    pub fn of_row(path: &Path) -> Option<Place> {
        if !path.as_os_str().as_encoded_bytes().starts_with(b"\0") {
            return None;
        }
        (desktop_places().into_iter()).find(|p| p.row_path().as_deref() == Some(path))
    }

    pub fn icon_name(&self) -> &'static str {
        match self {
            Place::Folder(_) => "folder",
            Place::Desktop => "user-desktop",
            Place::Documents => "folder-documents",
            Place::Home => "user-home",
            Place::Computer => "computer",
            Place::RecycleBin => "user-trash",
            Place::Network(uri) if uri == ROOT => "network-workgroup",
            Place::Network(_) => "network-server",
        }
    }

    pub fn shell(&self) -> Option<reprisefs::places::ShellPlace> {
        use reprisefs::places::ShellPlace;
        match self {
            Place::Documents => Some(ShellPlace::Documents),
            Place::Computer => Some(ShellPlace::Computer),
            Place::Home => Some(ShellPlace::Home),
            Place::RecycleBin => Some(ShellPlace::RecycleBin),
            Place::Folder(_) | Place::Desktop | Place::Network(_) => None,
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
            Place::Network(uri) if uri != ROOT => Some(Place::network()),
            Place::Documents
            | Place::Home
            | Place::Computer
            | Place::RecycleBin
            | Place::Network(_) => Some(Place::Desktop),
            Place::Desktop => None,
        }
    }

    pub fn parent(&self) -> Option<Place> {
        self.parent_in(&[])
    }

    pub fn read_only(&self) -> bool {
        matches!(
            self,
            Place::Computer | Place::RecycleBin | Place::Network(_)
        )
    }

    /// Whether the Details pane calls this a "System Folder": every shell place,
    /// including the three that are real writable directories.
    pub fn is_system(&self) -> bool {
        !matches!(self, Place::Folder(_))
    }

    /// Typed address-bar text or a command-line place: "Computer", "Recycle Bin" and "Network"
    /// (case-insensitive), the text the address bar shows for `here`, a mounted share's or
    /// device's name with a path below it, `~`/`~/...`, or an absolute existing directory.
    pub fn parse_address(s: &str, here: Option<&Place>) -> Option<Place> {
        let (place, dir) = Self::read_address(s, here, &[])?;
        dir.is_none_or(|d| d.is_dir()).then_some(place)
    }

    /// [`Place::parse_address`] without touching the file system, `shares` read by name: the
    /// place, and the folder still to be checked to exist when the text is a path.
    pub fn read_address(
        s: &str,
        here: Option<&Place>,
        shares: &[Share],
    ) -> Option<(Place, Option<PathBuf>)> {
        let s = s.trim();
        let lower = s.to_lowercase();
        for (name, place) in [
            ("computer", Place::Computer),
            ("recycle bin", Place::RecycleBin),
            ("network", Place::network()),
        ] {
            if lower == name {
                return Some((place, None));
            }
        }
        if let Some(here) = here.filter(|p| p.address_in(shares) == s) {
            return Some((here.clone(), None));
        }
        if let Some(dir) = named_path(s, shares) {
            return Some((Place::from_path(&dir), Some(dir)));
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

/// The shell places under the Desktop, in the tree's order (the address drop's and the Desktop
/// view's too), ahead of the Desktop folder's own items.
pub fn desktop_places() -> [Place; 5] {
    [
        Place::Documents,
        Place::Home,
        Place::Computer,
        Place::network(),
        Place::RecycleBin,
    ]
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

/// Mount points of the drives and network shares Computer lists. A subvolume on the root's
/// device is not a drive.
pub fn drive_mounts(drives: &[Drive], shares: &[Share]) -> Vec<PathBuf> {
    (drives.iter().map(|d| d.mount.clone()))
        .chain(shares.iter().map(|s| s.path.clone()))
        .collect()
}

/// A tree node's child folders: `listed`'s directories (a Desktop row as its place) minus any
/// that is a drive's mount point; a drive shows once, under Computer.
pub fn tree_subfolders(listed: &Listed, mounts: &[PathBuf]) -> Vec<(String, Place)> {
    listed
        .items
        .iter()
        .filter(|it| it.is_folder() && !mounts.iter().any(|m| m == &it.path))
        .map(|it| (it.name.clone(), Place::from_path(&it.path)))
        .collect()
}

/// Computer's children in the Folders tree: the mounted drives, then the mounted phones and
/// cameras and the network shares. An unmounted volume or device has no folder yet, and Home
/// hangs off the Desktop ([`Place::chain_in`]), so neither is a node here.
pub fn computer_tree_children(listed: &Listed) -> Vec<(String, Place)> {
    let shares = (listed.items.iter())
        .filter(|it| match it.drive {
            Some(DriveKind::Network) => true,
            Some(DriveKind::Portable(_)) => it.path.is_absolute(),
            _ => false,
        })
        .map(|it| (it.name.clone(), Place::Folder(it.path.clone())));
    (listed.drives.iter())
        .map(|d| (d.name(), Place::Folder(d.mount.clone())))
        .chain(shares)
        .collect()
}

/// The mounted share behind Computer's row `it`.
pub fn row_share<'a>(shares: &'a [Share], it: &Item) -> Option<&'a Share> {
    (it.drive == Some(DriveKind::Network))
        .then(|| shares.iter().find(|s| s.path == it.path))
        .flatten()
}

/// The phone or camera behind Computer's row `it`, among `devices` joined with their mounts
/// ([`portable::with_mounts`]): mounted at its path, or unmounted with its URI as the path.
pub fn row_device<'a>(devices: &'a [Device], it: &Item) -> Option<&'a Device> {
    matches!(it.drive, Some(DriveKind::Portable(_)))
        .then(|| {
            devices.iter().find(|d| match &d.mount {
                Some(m) => *m == it.path,
                None => Path::new(&d.uri) == it.path,
            })
        })
        .flatten()
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

/// What Eject on row `it` removes: its volume and the removable drive holding it, or no drive for
/// a disk image. `None` for a fixed disk or a drive UDisks2 does not know.
pub fn eject_target<'a>(
    volumes: &'a Snapshot,
    it: &Item,
) -> Option<(&'a Volume, Option<&'a Disk>)> {
    let v = row_volume(volumes, it).filter(|v| volumes.removable(v))?;
    match (volumes.disk_of(v), &v.image) {
        (None, None) => None,
        (disk, _) => Some((v, disk)),
    }
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
/// by name; descending reverses that. The Desktop's rows ([`Place::of_row`]) lead either way, in
/// the tree's order. Keys are built once per item.
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
    order.sort_by_key(|&i| row_rank(&items[i]));
    order
}

/// `it`'s place among [`desktop_places`] when it is a Desktop row, else past them all.
fn row_rank(it: &Item) -> usize {
    (Place::of_row(&it.path))
        .and_then(|p| desktop_places().iter().position(|d| *d == p))
        .unwrap_or(usize::MAX)
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

/// What Computer lists besides the folders: the drives, UDisks2's volumes, gvfs's mounts (network
/// shares, phones and cameras) and the phones and cameras plugged in.
#[derive(Clone, Copy)]
pub struct Devices<'a> {
    pub drives: &'a [Drive],
    pub volumes: &'a Snapshot,
    pub shares: &'a [Share],
    pub portable: &'a [Device],
}

/// `place`'s contents: a real directory read (for `RecycleBin` each of
/// [`bin_dirs`]) filtered by `hidden` unless `show_hidden`; `Computer` builds its
/// groups from `devices`; `Network` asks gvfs and Avahi ([`browse::list`]).
pub fn list(
    place: &Place,
    globs: &reprisefs::apps::MimeDb,
    show_hidden: bool,
    devices: Devices,
) -> Listed {
    match place {
        Place::Computer => list_computer(devices),
        Place::Network(uri) => {
            let entries = browse::list(uri).unwrap_or_else(|e| {
                log::info!("explorer: listing {uri}: {e}");
                Vec::new()
            });
            Listed {
                items: entries.iter().map(network_item).collect(),
                groups: Vec::new(),
                drives: Vec::new(),
            }
        }
        Place::RecycleBin => {
            let items: Vec<Item> = bin_dirs()
                .iter()
                .flat_map(|dir| reprisefs::listing::read_bin(dir, globs))
                .filter(|i| show_hidden || !i.hidden)
                .collect();
            Listed {
                items,
                groups: Vec::new(),
                drives: Vec::new(),
            }
        }
        Place::Desktop => desktop_listing(place.path().as_deref(), globs, show_hidden),
        _ => {
            let Some(dir) = place.path() else {
                return empty_listed();
            };
            Listed {
                items: read_folder(&dir, globs, show_hidden),
                groups: Vec::new(),
                drives: Vec::new(),
            }
        }
    }
}

fn read_folder(dir: &Path, globs: &reprisefs::apps::MimeDb, show_hidden: bool) -> Vec<Item> {
    let mut items = reprisefs::listing::read_dir(dir, globs);
    if !show_hidden {
        items.retain(|i| !i.hidden);
    }
    items
}

/// The Desktop: a row for each of [`desktop_places`], then the Desktop folder `dir`'s items.
pub(crate) fn desktop_listing(
    dir: Option<&Path>,
    globs: &reprisefs::apps::MimeDb,
    show_hidden: bool,
) -> Listed {
    let mut items: Vec<Item> = desktop_places().iter().filter_map(row_item).collect();
    if let Some(dir) = dir {
        items.extend(read_folder(dir, globs, show_hidden));
    }
    Listed {
        items,
        groups: Vec::new(),
        drives: Vec::new(),
    }
}

fn row_item(place: &Place) -> Option<Item> {
    Some(Item {
        name: place.title(),
        path: place.row_path()?,
        is_dir: true,
        is_link: false,
        exec: false,
        program: None,
        hidden: false,
        size: 0,
        modified: None,
        mime: None,
        kind: "System Folder".to_string(),
        drive: None,
        mount_root: false,
        trashed: None,
        shortcut: None,
    })
}

fn drive_item(d: &Drive) -> Item {
    Item {
        name: d.name(),
        path: d.mount.clone(),
        is_dir: true,
        is_link: false,
        exec: false,
        program: None,
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
    let optical = matches!(v.fs_type.as_str(), "iso9660" | "udf");
    let kind = if disk.is_some_and(|d| d.optical) || (v.image.is_some() && optical) {
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
        program: None,
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

fn list_computer(d: Devices) -> Listed {
    build_computer(
        d.drives.to_vec(),
        d.volumes,
        d.shares,
        d.portable,
        home().map(|h| (h, home_label())),
    )
}

/// A row standing for something with no folder of its own to read: a network share, or a server
/// or share in the Network place, whose path is its address.
fn remote_item(name: &str, path: PathBuf, kind: DriveKind) -> Item {
    Item {
        name: name.to_string(),
        path,
        is_dir: true,
        is_link: false,
        exec: false,
        program: None,
        hidden: false,
        size: 0,
        modified: None,
        mime: None,
        kind: kind.label().to_string(),
        drive: Some(kind),
        mount_root: true,
        trashed: None,
        shortcut: None,
    }
}

fn network_item(e: &browse::Entry) -> Item {
    let kind = match e.kind {
        browse::Kind::Server => DriveKind::Server,
        browse::Kind::Share => DriveKind::Network,
    };
    remote_item(&e.name, PathBuf::from(&e.uri), kind)
}

/// Computer: "Files Stored on This Computer" (the home folder `home`),
/// "Hard Disk Drives" (non-removable, non-optical), "Devices with Removable
/// Storage" (removable or optical drives, then unmounted removable `volumes`),
/// "Portable Devices" (the phones and cameras plugged in or mounted), then
/// "Network Locations" (the other mounted `shares`), each omitted when empty.
/// Split from `list_computer` so a test can drive it with fake drives.
fn build_computer(
    drives: Vec<Drive>,
    volumes: &Snapshot,
    shares: &[Share],
    portable: &[Device],
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
            program: None,
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
    let portable = portable::with_mounts(portable, shares);
    if !portable.is_empty() {
        groups.push(("Portable Devices".to_string(), items.len()));
        items.extend(portable.iter().map(|d| {
            let path = d.mount.clone().unwrap_or_else(|| PathBuf::from(&d.uri));
            remote_item(&d.name, path, DriveKind::Portable(d.kind))
        }));
    }
    let shares: Vec<&Share> = shares.iter().filter(|s| !s.is_portable()).collect();
    if !shares.is_empty() {
        groups.push(("Network Locations".to_string(), items.len()));
        items.extend(
            (shares.iter()).map(|s| remote_item(&s.name, s.path.clone(), DriveKind::Network)),
        );
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
        assert_eq!(Place::Documents.address_in(&[]), "Documents");
        assert_eq!(Place::Home.icon_name(), "user-home");
        // A listing shows the home folder with Home's icon too.
        assert_eq!(
            reprisefs::places::special_folders().first(),
            super::home().map(|h| (h, "user-home")).as_ref()
        );
    }

    #[test]
    fn a_folder_inside_a_mount_is_addressed_by_its_name_and_read_back() {
        let root = PathBuf::from("/run/user/1000/gvfs/mtp:host=Reprise_Test_Phone");
        let names = [Share {
            name: "Test Phone".into(),
            icon: String::new(),
            path: root.clone(),
            backend: "mtp".into(),
            host: "Reprise_Test_Phone".into(),
        }];
        let inner = root.join("Internal shared storage/DCIM");
        assert_eq!(named_address(&root, &names), "Test Phone");
        assert_eq!(
            named_address(&inner, &names),
            "Test Phone/Internal shared storage/DCIM"
        );
        assert_eq!(
            named_address(Path::new("/run/user/1000/gvfs"), &names),
            "/run/user/1000/gvfs",
            "gvfs's own folder keeps its path"
        );
        assert_eq!(named_path("Test Phone", &names), Some(root.clone()));
        assert_eq!(
            named_path("Test Phone/Internal shared storage/DCIM", &names),
            Some(inner)
        );
        assert_eq!(named_path("Test Phones", &names), None);
        assert_eq!(named_path("/run/user/1000/gvfs", &names), None);
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
            &[share()],
            &[],
            Some((PathBuf::from("/home/u"), "u's Home".to_string())),
        );
        let kids = computer_tree_children(&listed);
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
                ("public on nas".to_string(), Place::Folder(share().path)),
            ],
            "the drives, named as the view names them, then the shares, and not the Home the view lists"
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
            program: None,
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
            &[share()],
            &[],
            Some((PathBuf::from("/home/u"), "u's Home".to_string())),
        );

        assert_eq!(
            listed.groups,
            vec![
                ("Files Stored on This Computer".to_string(), 0),
                ("Hard Disk Drives".to_string(), 1),
                ("Devices with Removable Storage".to_string(), 2),
                ("Network Locations".to_string(), 3),
            ]
        );
        let share_row = &listed.items[3];
        assert_eq!(share_row.name, "public on nas");
        assert_eq!(share_row.kind, "Network Drive");
        assert!(
            share_row.mount_root,
            "a share is never read into: its server may not answer"
        );
        assert_eq!(row_share(&[share()], share_row), Some(&share()));
        assert_eq!(row_share(&[share()], &listed.items[1]), None);
        assert_eq!(listed.items[0].name, "u's Home");
        assert_eq!(listed.items[1].name, "Local Disk (/)");
        assert_eq!(listed.items[1].kind, "Local Disk");
        assert_eq!(listed.items[2].name, "MYDVD (/run/media/u/DVD)");
        assert_eq!(listed.items[2].kind, "CD Drive");
        assert_eq!(listed.items[0].drive, None);
        assert_eq!(listed.items[2].drive, Some(DriveKind::Optical));
        assert_eq!(listed.drives.len(), 2);
    }

    fn share() -> Share {
        Share {
            name: "public on nas".into(),
            icon: "folder-remote".into(),
            path: PathBuf::from("/run/user/1000/gvfs/smb-share:server=nas,share=public"),
            backend: "smb-share".into(),
            host: "nas".into(),
        }
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
        let listed = build_computer(drives, &udisks_snapshot(), &[], &[], None);

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
            program: None,
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
            eject_target(&snap, &card).map(|(v, d)| (v.device.as_str(), d.unwrap().path.as_str())),
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
    fn a_mounted_disk_image_is_listed_with_the_removable_drives_and_ejects_without_a_drive() {
        let mut snap = Snapshot::default();
        snap.volumes.push(Volume {
            path: "/loop0".into(),
            device: "/dev/loop0".into(),
            label: Some("DEBIAN_13".into()),
            fs_type: "iso9660".into(),
            filesystem: true,
            mounts: vec![PathBuf::from("/run/media/u/DEBIAN_13")],
            system: true,
            image: Some(reprisefs::udisks::Image {
                file: "/home/u/debian.iso".into(),
                loop_path: "/loop0".into(),
            }),
            ..Volume::default()
        });
        let listed = build_computer(snap.image_drives(), &snap, &[], &[], None);
        assert_eq!(
            listed.groups,
            [("Devices with Removable Storage".to_string(), 0)]
        );
        let row = &listed.items[0];
        assert_eq!(row.drive, Some(DriveKind::Optical));
        let (v, disk) = eject_target(&snap, row).expect("Eject");
        assert_eq!((v.device.as_str(), disk), ("/dev/loop0", None));
    }

    fn phone() -> Device {
        Device {
            name: "Pixel 7".into(),
            kind: reprisefs::places::Gadget::Phone,
            uri: "mtp://Google_Pixel_7_2A0F/".into(),
            mount: None,
        }
    }

    fn phone_mount() -> Share {
        Share {
            name: "Pixel 7".into(),
            icon: "phone".into(),
            path: PathBuf::from("/run/user/1000/gvfs/mtp:host=Google_Pixel_7_2A0F"),
            backend: "mtp".into(),
            host: "Google_Pixel_7_2A0F".into(),
        }
    }

    #[test]
    fn a_plugged_phone_lists_under_portable_devices_by_its_uri_until_mounted() {
        let listed = build_computer(
            Vec::new(),
            &Snapshot::default(),
            &[share()],
            &[phone()],
            None,
        );
        assert_eq!(
            listed.groups,
            vec![
                ("Portable Devices".to_string(), 0),
                ("Network Locations".to_string(), 1),
            ]
        );
        let row = &listed.items[0];
        assert_eq!(row.name, "Pixel 7");
        assert_eq!(row.kind, "Mobile Phone");
        assert_eq!(row.path, PathBuf::from("mtp://Google_Pixel_7_2A0F/"));
        assert!(row.mount_root, "a device is never read into while listing");
        assert_eq!(row_device(&[phone()], row), Some(&phone()));
        assert_eq!(row_share(&[share()], row), None, "a device is no share");
        assert!(
            computer_tree_children(&listed)
                .iter()
                .all(|(n, _)| n != "Pixel 7")
        );
    }

    #[test]
    fn a_mounted_phone_opens_as_its_fuse_folder_and_leaves_network_locations() {
        let shares = [share(), phone_mount()];
        let listed = build_computer(Vec::new(), &Snapshot::default(), &shares, &[phone()], None);
        let row = &listed.items[0];
        assert_eq!(row.path, phone_mount().path);
        assert_eq!(listed.items.len(), 2, "the phone's mount shows once");
        assert_eq!(listed.items[1].name, "public on nas");
        let joined = portable::with_mounts(&[phone()], &shares);
        assert_eq!(
            row_device(&joined, row).and_then(|d| d.mount.clone()),
            Some(phone_mount().path)
        );
        assert_eq!(
            computer_tree_children(&listed)[0],
            ("Pixel 7".to_string(), Place::Folder(phone_mount().path))
        );
    }

    #[test]
    fn build_computer_omits_empty_groups() {
        let listed = build_computer(Vec::new(), &Snapshot::default(), &[], &[], None);
        assert!(listed.groups.is_empty());
        assert!(listed.items.is_empty());
    }

    /// The Desktop folder holding the folder `Projects` and the file `notes.txt`.
    fn desktop_folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::fs::create_dir(dir.path().join("Projects")).expect("mkdir");
        std::fs::write(dir.path().join("notes.txt"), "x").expect("write");
        dir
    }

    #[test]
    fn the_desktop_lists_a_row_per_shell_place_then_its_folder() {
        let dir = desktop_folder();
        let listed = desktop_listing(Some(dir.path()), &reprisefs::apps::MimeDb::empty(), true);
        let rows: Vec<Place> = (listed.items.iter())
            .map_while(|it| Place::of_row(&it.path))
            .collect();
        assert_eq!(rows, desktop_places(), "the rows lead, in the tree's order");
        let names: Vec<&str> = listed.items.iter().map(|it| it.label()).collect();
        let home = home_label();
        assert_eq!(
            names[..5],
            [
                "Documents",
                home.as_str(),
                "Computer",
                "Network",
                "Recycle Bin"
            ]
        );
        let mut files = names[5..].to_vec();
        files.sort();
        assert_eq!(files, ["Projects", "notes.txt"]);
        for it in &listed.items[..5] {
            assert!(
                it.is_folder() && !it.hidden,
                "{} lists as a folder",
                it.name
            );
            assert_eq!(it.kind, "System Folder");
            assert_eq!(Some(Place::from_path(&it.path)), Place::of_row(&it.path));
        }
        let places: Vec<Place> = (tree_subfolders(&listed, &[]).into_iter())
            .map(|(_, p)| p)
            .collect();
        let mut tree = desktop_places().to_vec();
        tree.push(Place::from_path(&dir.path().join("Projects")));
        assert_eq!(
            places, tree,
            "the tree's Desktop node holds the view's folders"
        );
    }

    #[test]
    fn a_desktop_rows_path_is_refused_by_the_file_system() {
        for place in desktop_places() {
            let path = place.row_path().expect("a row");
            let refused = |e: std::io::Error| e.kind() == std::io::ErrorKind::InvalidInput;
            assert!(
                std::fs::metadata(&path).is_err_and(refused),
                "{path:?} is stat'ed"
            );
            assert!(
                std::fs::create_dir_all(&path).is_err_and(refused),
                "{path:?} is made"
            );
            assert!(
                std::fs::remove_file(&path).is_err_and(refused),
                "{path:?} is removed"
            );
        }
        assert_eq!(Place::Desktop.row_path(), None);
        assert_eq!(Place::Folder(PathBuf::from("/tmp")).row_path(), None);
        assert_eq!(Place::of_row(Path::new("/tmp/computer")), None);
    }

    #[test]
    fn the_desktop_rows_lead_whichever_way_it_sorts() {
        let dir = desktop_folder();
        let listed = desktop_listing(Some(dir.path()), &reprisefs::apps::MimeDb::empty(), true);
        let mut items = listed.items;
        items.reverse();
        for (field, asc) in [
            (Field::Name, true),
            (Field::Name, false),
            (Field::Size, false),
        ] {
            let order = sort_order(&items, field, asc);
            let rows: Vec<Option<Place>> = (order[..5].iter())
                .map(|&i| Place::of_row(&items[i].path))
                .collect();
            assert_eq!(
                rows,
                desktop_places().map(Some),
                "{field:?} {asc}: the rows first, in the tree's order"
            );
        }
        let by_name = |asc| {
            let order = sort_order(&items, Field::Name, asc);
            order[5..]
                .iter()
                .map(|&i| items[i].name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(by_name(true), ["Projects", "notes.txt"]);
        assert_eq!(by_name(false), ["notes.txt", "Projects"]);
    }
}
