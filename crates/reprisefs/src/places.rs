//! Places: the freedesktop trash, the user's home-relative directories and
//! mounted removable drives.

use std::collections::HashSet;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

/// The freedesktop trash, `$XDG_DATA_HOME/Trash` (or `~/.local/share/Trash`), holding `files/` and `info/`.
pub fn trash_root() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(data.join("Trash"))
}

pub fn trash_dir() -> Option<PathBuf> {
    Some(trash_root()?.join("files"))
}

/// The folders to watch so the trash at `root` is seen filling, arriving and leaving: its files
/// folder (every entry), then each existing folder above it up to `root`'s parent, for the one
/// entry on the way down. A missing parent is stood in for by its nearest existing ancestor, so a
/// first trashing on a fresh account is seen.
pub fn trash_watches(root: &Path) -> Vec<(PathBuf, Option<OsString>)> {
    let files = root.join("files");
    let mut out = Vec::new();
    if files.is_dir() {
        out.push((files.clone(), None));
    }
    let mut child = files.as_path();
    while let Some(dir) = child.parent() {
        if dir.is_dir() {
            out.push((dir.to_path_buf(), child.file_name().map(OsString::from)));
            if !dir.starts_with(root) {
                break;
            }
        }
        child = dir;
    }
    out
}

/// Whether the trash's files folder has anything in it; cheaper than the count `trash_contents` takes.
pub fn trash_empty(files: &Path) -> bool {
    match std::fs::read_dir(files) {
        Ok(mut rd) => rd.next().is_none(),
        Err(_) => true,
    }
}

/// The count of top-level entries in the trash's files folder and the name of the first, for the Recycle Bin's icon and Empty confirmation.
pub fn trash_contents(files: &Path) -> (usize, Option<String>) {
    let Ok(rd) = std::fs::read_dir(files) else {
        return (0, None);
    };
    let mut count = 0;
    let mut first: Option<String> = None;
    for e in rd.flatten() {
        count += 1;
        let name = e.file_name().to_string_lossy().into_owned();
        if first.as_ref().is_none_or(|f| name < *f) {
            first = Some(name);
        }
    }
    (count, first)
}

/// `$XDG_CONFIG_HOME/user-dirs.dirs`, which names the user's folders.
pub fn user_dirs_file() -> PathBuf {
    crate::autostart::config_home().join("user-dirs.dirs")
}

/// `XDG_<key>_DIR` from `$XDG_CONFIG_HOME/user-dirs.dirs`, else `$HOME/<fallback>`; `None` without a home.
pub fn user_dir(key: &str, fallback: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let user_dirs = user_dirs_file();
    match std::fs::read_to_string(&user_dirs) {
        Ok(text) => {
            if let Some(p) = parse_user_dir(&text, &home, key) {
                return Some(p);
            }
            log::debug!(
                "{}: no absolute XDG_{key}_DIR, falling back to ~/{fallback}",
                user_dirs.display()
            );
        }
        Err(e) => {
            log::debug!("{}: {e}, falling back to ~/{fallback}", user_dirs.display());
        }
    }
    Some(home.join(fallback))
}

pub fn parse_user_dir(text: &str, home: &Path, key: &str) -> Option<PathBuf> {
    let prefix = format!("XDG_{key}_DIR=");
    for line in text.lines() {
        let line = line.trim();
        let Some(value) = line.strip_prefix(prefix.as_str()) else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        let expanded = match value.strip_prefix("$HOME") {
            Some(rest) => format!("{}{}", home.display(), rest),
            None => value.to_string(),
        };
        let p = PathBuf::from(expanded);
        return if p.is_absolute() { Some(p) } else { None };
    }
    None
}

/// The folders a listing gives their own icon: home (`user-home`) and [`document_dir_paths`].
pub fn special_folders() -> Vec<(PathBuf, &'static str)> {
    let home = std::env::var_os("HOME").map(|h| (PathBuf::from(h), "user-home"));
    let mut out: Vec<(PathBuf, &'static str)> = home.into_iter().collect();
    out.extend(document_dir_paths());
    out
}

/// The user directories with their icon names, whether or not they exist, from one read and no
/// stat: the Start menus' document folders, and what a listing matches its folders against.
pub fn document_dir_paths() -> Vec<(PathBuf, &'static str)> {
    const DIRS: [(&str, &str, &str); 5] = [
        ("DOCUMENTS", "Documents", "folder-documents"),
        ("PICTURES", "Pictures", "folder-pictures"),
        ("MUSIC", "Music", "folder-music"),
        ("VIDEOS", "Videos", "folder-videos"),
        ("DOWNLOAD", "Downloads", "folder-download"),
    ];
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let text = std::fs::read_to_string(user_dirs_file()).unwrap_or_default();
    DIRS.iter()
        .map(|(key, fallback, icon)| {
            let dir = parse_user_dir(&text, &home, key).unwrap_or_else(|| home.join(fallback));
            (dir, *icon)
        })
        .collect()
}

/// The folder path the My Recent Documents tile stands for. It is no real
/// folder (every listed folder is absolute): [`is_recent`] tells it apart and
/// `listing::dialog_rows` lists the recently used files for it.
pub const RECENT: &str = "recent:";

pub fn is_recent(p: &Path) -> bool {
    p == Path::new(RECENT)
}

/// The folder a dialog's Up goes to; None at `/` and in My Recent Documents.
pub fn dialog_parent(dir: &Path) -> Option<&Path> {
    if is_recent(dir) { None } else { dir.parent() }
}

pub fn dialog_label(dir: &Path) -> String {
    if is_recent(dir) {
        return "My Recent Documents".to_string();
    }
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.display().to_string())
}

#[derive(Clone, Debug, PartialEq)]
pub struct DialogPlace {
    pub label: String,
    pub icon: &'static str,
    /// None without a home directory: drawn greyed and never entered.
    pub dir: Option<PathBuf>,
}

/// "<user>'s Home", or "Home" without `$USER`.
pub fn home_label() -> String {
    match std::env::var("USER") {
        Ok(user) if !user.is_empty() => format!("{user}'s Home"),
        _ => "Home".to_string(),
    }
}

/// The Open and Save dialogs' places bar, top to bottom: My Recent
/// Documents, home ([`home_label`]), Desktop, Documents, Downloads and
/// Computer (`/`). Shared by every program's dialog and the compositor's
/// Browse. Without `recent` (`[privacy] recent_files = false`) the My Recent
/// Documents tile is greyed.
pub fn dialog_places(recent: bool) -> Vec<DialogPlace> {
    let home_label = home_label();
    let place = |label: &str, icon, dir| DialogPlace {
        label: label.to_string(),
        icon,
        dir,
    };
    vec![
        place(
            "My Recent Documents",
            "document-open-recent",
            recent.then(|| PathBuf::from(RECENT)),
        ),
        place(
            &home_label,
            "user-home",
            std::env::var_os("HOME").map(PathBuf::from),
        ),
        place("Desktop", "user-desktop", user_dir("DESKTOP", "Desktop")),
        place(
            "Documents",
            "folder-documents",
            user_dir("DOCUMENTS", "Documents"),
        ),
        place(
            "Downloads",
            "folder-download",
            user_dir("DOWNLOAD", "Downloads"),
        ),
        place("Computer", "computer", Some(PathBuf::from("/"))),
    ]
}

/// Writable mount points under `/run/media/<user>` and `/media`, sorted by name.
/// Read-only mounts are left out rather than offered and failed, and so are
/// network mounts: a `stat` on a wedged one blocks the caller's loop. A folder
/// there that is no mount point (one left behind by an unmount) is no drive.
pub fn mounted_drives() -> Vec<PathBuf> {
    MountedDrives::new().current().to_vec()
}

/// [`mounted_drives`] for a long-lived process: `listed` walks the mount roots again only when the
/// mount table changed.
#[derive(Debug)]
pub struct MountedDrives {
    mounts_file: PathBuf,
    roots: Vec<PathBuf>,
    mounts: Option<Vec<u8>>,
    listed: Vec<PathBuf>,
    /// `listed` filtered by [`usable_drive`], None until `current` asks after a table change.
    drives: Option<Vec<PathBuf>>,
}

impl MountedDrives {
    pub fn new() -> MountedDrives {
        let mut roots = vec![PathBuf::from("/media")];
        if let Some(user) = std::env::var_os("USER") {
            roots.insert(0, PathBuf::from("/run/media").join(user));
        }
        MountedDrives::at("/proc/self/mounts".into(), roots)
    }

    pub fn at(mounts_file: PathBuf, roots: Vec<PathBuf>) -> MountedDrives {
        MountedDrives {
            mounts_file,
            roots,
            mounts: None,
            listed: Vec::new(),
            drives: None,
        }
    }

    /// The local mount points under the roots, sorted, before [`usable_drive`] looks at them:
    /// nothing here touches a drive, so a drive that never answers cannot hold it.
    pub fn listed(&mut self) -> &[PathBuf] {
        let mounts = std::fs::read(&self.mounts_file).unwrap_or_default();
        if self.mounts.as_ref() == Some(&mounts) {
            return &self.listed;
        }
        let remote = network_mounts(&mounts);
        let mounted: HashSet<PathBuf> = mount_table(&mounts).into_iter().map(|(p, _)| p).collect();
        self.listed = self
            .roots
            .iter()
            .filter_map(|r| std::fs::read_dir(r).ok())
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| mounted.contains(p) && !remote.contains(p))
            .collect();
        self.listed.sort();
        self.mounts = Some(mounts);
        self.drives = None;
        &self.listed
    }

    pub fn current(&mut self) -> &[PathBuf] {
        self.listed();
        let listed = &self.listed;
        self.drives
            .get_or_insert_with(|| listed.iter().filter(|p| usable_drive(p)).cloned().collect())
    }
}

/// Whether the mount point `top` is a folder the user may write to. It stats the drive, which
/// waits for as long as the drive does not answer.
pub fn usable_drive(top: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(top).is_ok_and(|m| m.is_dir() && m.permissions().mode() & 0o200 != 0)
}

impl Default for MountedDrives {
    fn default() -> MountedDrives {
        MountedDrives::new()
    }
}

/// The mount points in `mounts` (a `/proc/mounts` table) on a network file system.
pub fn network_mounts(mounts: impl AsRef<[u8]>) -> HashSet<PathBuf> {
    mount_lines(mounts.as_ref())
        .filter(|(_, _, fs)| is_network_fs(fs))
        .map(|(_, mount, _)| mount_path(mount))
        .collect()
}

/// Each mount point in `mounts` with its file system type, in table order, unescaped.
pub fn mount_table(mounts: &[u8]) -> Vec<(PathBuf, String)> {
    mount_lines(mounts)
        .map(|(_, mount, fs)| (mount_path(mount), fs))
        .collect()
}

/// Each mount point in `mounts` with its device, in table order, unescaped.
pub fn mount_devices(mounts: &[u8]) -> Vec<(PathBuf, String)> {
    mount_lines(mounts)
        .map(|(device, mount, _)| (mount_path(mount), mount_device(device)))
        .collect()
}

/// Whether `fs` can wait on a server, or is a kernel file system that never
/// holds the user's files (`proc`, `sysfs`, `autofs`...): a walk of every mount skips these.
pub fn unlisted_fs(fs: &str) -> bool {
    is_network_fs(fs)
        || matches!(
            fs,
            "proc"
                | "sysfs"
                | "devtmpfs"
                | "devpts"
                | "cgroup"
                | "cgroup2"
                | "securityfs"
                | "pstore"
                | "efivarfs"
                | "bpf"
                | "debugfs"
                | "tracefs"
                | "configfs"
                | "fusectl"
                | "mqueue"
                | "hugetlbfs"
                | "binfmt_misc"
                | "autofs"
                | "nsfs"
                | "selinuxfs"
                | "rpc_pipefs"
                | "squashfs"
        )
}

/// The device, mount point and file system fields of each line, still
/// escaped. The table is bytes: a mount point is any path the kernel accepts.
fn mount_lines(mounts: &[u8]) -> impl Iterator<Item = (&[u8], &[u8], String)> {
    mounts.split(|&b| b == b'\n').filter_map(|line| {
        let mut fields = line
            .split(u8::is_ascii_whitespace)
            .filter(|f| !f.is_empty());
        let (device, mount, fs) = (fields.next()?, fields.next()?, fields.next()?);
        Some((device, mount, String::from_utf8_lossy(fs).into_owned()))
    })
}

fn mount_path(field: &[u8]) -> PathBuf {
    PathBuf::from(OsString::from_vec(unescape_mount(field)))
}

/// The kernel names block devices in ASCII, so nothing is lost.
fn mount_device(field: &[u8]) -> String {
    String::from_utf8_lossy(&unescape_mount(field)).into_owned()
}

fn is_network_fs(fs: &str) -> bool {
    matches!(
        fs,
        "nfs" | "nfs4" | "cifs" | "smb3" | "afs" | "9p" | "ceph" | "glusterfs" | "ncpfs" | "afp"
    ) || matches!(
        fs,
        "fuse.sshfs" | "fuse.davfs2" | "fuse.gvfsd-fuse" | "fuse.rclone"
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drive {
    pub mount: PathBuf,
    pub device: String,
    pub fs: String,
    pub label: Option<String>,
    pub removable: bool,
    pub optical: bool,
    pub total: u64,
    pub free: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellPlace {
    Documents,
    Computer,
    Home,
    RecycleBin,
}

impl ShellPlace {
    pub fn tip(self) -> &'static str {
        match self {
            ShellPlace::Documents => "Keeps your documents, pictures and other files together.",
            ShellPlace::Computer => "Shows the drives and devices attached to this computer.",
            ShellPlace::Home => "Holds your own files and settings.",
            ShellPlace::RecycleBin => "Holds what you have deleted until you empty it.",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveKind {
    Local,
    Removable,
    Optical,
}

impl DriveKind {
    pub fn label(self) -> &'static str {
        match self {
            DriveKind::Local => "Local Disk",
            DriveKind::Removable => "Removable Disk",
            DriveKind::Optical => "CD Drive",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            DriveKind::Local => "drive-harddisk",
            DriveKind::Removable => "drive-removable-media",
            DriveKind::Optical => "media-optical",
        }
    }
}

impl Drive {
    pub fn kind(&self) -> DriveKind {
        if self.optical {
            DriveKind::Optical
        } else if self.removable {
            DriveKind::Removable
        } else {
            DriveKind::Local
        }
    }

    pub fn name(&self) -> String {
        let label = self.label.as_deref().unwrap_or(self.kind().label());
        format!("{label} ({})", self.mount.display())
    }
}

pub fn drives() -> Vec<Drive> {
    let mounts = std::fs::read("/proc/mounts").unwrap_or_default();
    drives_from(&mounts, &sys_removable, &disk_label, &|m| {
        statvfs_totals(m).unwrap_or((0, 0))
    })
}

fn sys_removable(basename: &str) -> bool {
    let disk = resolve_disk(basename);
    std::fs::read_to_string(format!("/sys/block/{disk}/removable")).is_ok_and(|s| s.trim() == "1")
}

/// The disk that owns `basename`: the parent of the `/sys/class/block/<basename>`
/// link target, `block` itself for a whole disk. Without `/sys`, [`disk_name`].
fn resolve_disk(basename: &str) -> String {
    match std::fs::read_link(format!("/sys/class/block/{basename}")) {
        Ok(target) => {
            match target
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
            {
                Some(name) if name != "block" => name.to_string(),
                _ => basename.to_string(),
            }
        }
        Err(_) => disk_name(basename),
    }
}

fn disk_label(device: &str) -> Option<String> {
    let dir = Path::new("/dev/disk/by-label");
    let target = Path::new(device).canonicalize().ok()?;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let link = entry.path();
        let Ok(dest) = std::fs::read_link(&link) else {
            continue;
        };
        let resolved = if dest.is_relative() {
            link.parent().map(|p| p.join(&dest)).unwrap_or(dest)
        } else {
            dest
        };
        if resolved.canonicalize().ok().as_deref() == Some(target.as_path()) {
            return entry.file_name().to_str().map(|s| s.to_string());
        }
    }
    None
}

/// `(total, free)` bytes; free is what an unprivileged caller may use.
pub fn statvfs_totals(path: &Path) -> Option<(u64, u64)> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: `c` is a valid NUL-terminated path and `buf` is a
    // correctly sized, writable `statvfs` that the call fills in.
    unsafe {
        let mut buf: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut buf) != 0 {
            return None;
        }
        Some((
            buf.f_blocks as u64 * buf.f_frsize as u64,
            buf.f_bavail as u64 * buf.f_frsize as u64,
        ))
    }
}

/// `basename` with its partition number stripped (`sda2` -> `sda`,
/// `nvme0n1p2` -> `nvme0n1`). A whole disk ending in a digit (`sr0`) reads as
/// a partition, safe only because optical drives never reach `sys_removable`.
fn disk_name(basename: &str) -> String {
    if let Some(p_idx) = basename.rfind('p') {
        let (head, tail) = basename.split_at(p_idx);
        let tail = &tail[1..];
        if !tail.is_empty()
            && tail.bytes().all(|b| b.is_ascii_digit())
            && head.bytes().next_back().is_some_and(|b| b.is_ascii_digit())
        {
            return head.to_string();
        }
    }
    basename
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .to_string()
}

/// True for a device or file system Computer does not show: loopback,
/// ramdisk, zram and network-block devices, and squashfs, overlay, tmpfs and fuse mounts.
fn is_pseudo(basename: &str, fs: &str) -> bool {
    const VIRTUAL_PREFIXES: [&str; 4] = ["loop", "ram", "zram", "nbd"];
    let virtual_device = VIRTUAL_PREFIXES.iter().any(|p| basename.starts_with(p));
    let virtual_fs = matches!(fs, "squashfs" | "overlay" | "tmpfs") || fs.starts_with("fuse");
    virtual_device || virtual_fs
}

/// Undo `/proc/mounts`' octal escaping of space, tab, newline and backslash
/// (`\040`, `\011`, `\012`, `\134`) in a device or mount point field.
fn unescape_mount(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        // Summed in u32: `\400` and up are not bytes, so any text passes through without overflow.
        let octal = |k: usize| (bytes[i + k] - b'0') as u32;
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1..i + 4].iter().all(|b| matches!(b, b'0'..=b'7'))
            && octal(1) * 64 + octal(2) * 8 + octal(3) <= 255
        {
            out.push((octal(1) * 64 + octal(2) * 8 + octal(3)) as u8);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

/// The drives in `mounts`: `/dev/` devices deduplicated by device (first
/// mount wins), minus `is_pseudo` ones. `sys_removable` answers for a device
/// basename, `labels` for a device and `stat` for a mount point. A mapper
/// device is never removable and an optical one always is. A `/` that is not
/// a block device (ZFS, NFS, a container's overlay) is added first, since
/// Computer is the only way into the file system.
pub fn drives_from(
    mounts: impl AsRef<[u8]>,
    sys_removable: &dyn Fn(&str) -> bool,
    labels: &dyn Fn(&str) -> Option<String>,
    stat: &dyn Fn(&Path) -> (u64, u64),
) -> Vec<Drive> {
    let mounts = mounts.as_ref();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (device_raw, mount_raw, fs) in mount_lines(mounts) {
        if !device_raw.starts_with(b"/dev/") {
            continue;
        }
        let device = mount_device(device_raw);
        if !seen.insert(device.clone()) {
            continue;
        }
        let basename = device.rsplit('/').next().unwrap_or(device.as_str());
        if is_pseudo(basename, &fs) {
            continue;
        }
        let is_mapper = device.starts_with("/dev/mapper/") || device.starts_with("/dev/dm-");
        let optical = fs == "iso9660" || fs == "udf" || basename.starts_with("sr");
        let removable = if is_mapper {
            false
        } else if optical {
            true
        } else {
            sys_removable(basename)
        };
        let mount = mount_path(mount_raw);
        let (total, free) = stat(&mount);
        let label = labels(&device);
        out.push(Drive {
            mount,
            device,
            fs,
            label,
            removable,
            optical,
            total,
            free,
        });
    }
    // The Folders tree walks every absolute path up to `/`, so a root that failed the tests above is put back.
    if !out.iter().any(|d| d.mount == Path::new("/"))
        && let Some((device, fs)) = root_of(mounts)
    {
        let (total, free) = stat(Path::new("/"));
        out.insert(
            0,
            Drive {
                mount: PathBuf::from("/"),
                label: labels(&device),
                device,
                fs,
                // Nothing backing a root filesystem is removable.
                removable: false,
                optical: false,
                total,
                free,
            },
        );
    }
    out
}

/// The last device and file system `mounts` gives `/`: the initramfs writes a `rootfs` line that the real root mounts over.
fn root_of(mounts: &[u8]) -> Option<(String, String)> {
    mount_lines(mounts)
        .filter(|(_, mount, _)| unescape_mount(mount) == b"/")
        .map(|(device, _, fs)| (mount_device(device), fs))
        .last()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_trash_is_watched_from_its_nearest_existing_folder() {
        let home = tempfile::tempdir().unwrap();
        let data = home.path().join(".local/share");
        let root = data.join("Trash");
        let files = root.join("files");
        let at = |dir: &Path, name: Option<&str>| (dir.to_path_buf(), name.map(OsString::from));

        assert_eq!(
            trash_watches(&root),
            [at(home.path(), Some(".local"))],
            "no data folder yet: the home folder, for .local"
        );
        std::fs::create_dir_all(&data).unwrap();
        assert_eq!(
            trash_watches(&root),
            [at(&data, Some("Trash"))],
            "no trash yet: the data folder, for Trash"
        );
        std::fs::create_dir(&root).unwrap();
        assert_eq!(
            trash_watches(&root),
            [at(&root, Some("files")), at(&data, Some("Trash"))],
            "no files folder yet: the trash for it, the data folder for the trash leaving"
        );
        std::fs::create_dir(&files).unwrap();
        assert_eq!(
            trash_watches(&root),
            [
                at(&files, None),
                at(&root, Some("files")),
                at(&data, Some("Trash"))
            ],
        );
    }

    #[test]
    fn a_drive_is_named_and_iconed_by_what_it_is() {
        let mut d = Drive {
            mount: PathBuf::from("/"),
            device: "/dev/sda2".into(),
            fs: "ext4".into(),
            label: None,
            removable: false,
            optical: false,
            total: 0,
            free: 0,
        };
        assert_eq!(d.name(), "Local Disk (/)");
        assert_eq!(d.kind().icon(), "drive-harddisk");
        d.removable = true;
        assert_eq!(d.name(), "Removable Disk (/)");
        assert_eq!(d.kind().icon(), "drive-removable-media");
        d.optical = true;
        d.mount = PathBuf::from("/run/media/u/DVD");
        assert_eq!(d.name(), "CD Drive (/run/media/u/DVD)");
        assert_eq!(d.kind().icon(), "media-optical");
        d.label = Some("MYDVD".into());
        assert_eq!(d.name(), "MYDVD (/run/media/u/DVD)");
    }

    #[test]
    fn network_mounts_names_the_remote_mount_points_unescaped() {
        let mounts = "\
/dev/sda1 / ext4 rw 0 0
server:/export /media/nas nfs4 rw 0 0
//box/share /run/media/u/My\\040Share cifs rw 0 0
sshfs#u@h:/ /media/ssh fuse.sshfs rw 0 0
/dev/sdb1 /media/usb vfat rw 0 0
";
        let remote = network_mounts(mounts);
        assert_eq!(remote.len(), 3);
        assert!(remote.contains(Path::new("/media/nas")));
        assert!(remote.contains(Path::new("/run/media/u/My Share")));
        assert!(remote.contains(Path::new("/media/ssh")));
        assert!(!remote.contains(Path::new("/media/usb")));
    }

    #[test]
    fn mounted_drives_walk_again_when_the_mount_table_changes() {
        let t = tempfile::tempdir().unwrap();
        let (root, table) = (t.path().join("media"), t.path().join("mounts"));
        std::fs::create_dir_all(root.join("STICK")).unwrap();
        let line =
            |dev: &str, name: &str| format!("{dev} {} vfat rw 0 0\n", root.join(name).display());
        let stick = line("/dev/sdb1", "STICK");
        std::fs::write(&table, &stick).unwrap();
        let mut drives = MountedDrives::at(table.clone(), vec![root.clone()]);
        assert_eq!(drives.current(), [root.join("STICK")]);
        std::fs::create_dir(root.join("CARD")).unwrap();
        assert_eq!(drives.current(), [root.join("STICK")]);
        std::fs::write(&table, stick.clone() + &line("/dev/sdc1", "CARD")).unwrap();
        assert_eq!(drives.current(), [root.join("CARD"), root.join("STICK")]);
        std::fs::write(&table, line("/dev/sdc1", "CARD")).unwrap();
        assert_eq!(
            drives.current(),
            [root.join("CARD")],
            "STICK unmounted; its folder is left behind"
        );
    }

    #[test]
    fn trash_empty_is_the_non_empty_bit_and_true_for_a_missing_folder() {
        let dir = std::env::temp_dir().join(format!("reprise-trash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(trash_empty(&dir));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(trash_empty(&dir));
        std::fs::write(dir.join("a"), "").unwrap();
        assert!(!trash_empty(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn trash_contents_counts_items_and_names_the_first_in_sort_order() {
        let dir = std::env::temp_dir().join(format!("reprise-trashc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(trash_contents(&dir), (0, None));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["b", "c", "a"] {
            std::fs::write(dir.join(name), "").unwrap();
        }
        assert_eq!(trash_contents(&dir), (3, Some("a".to_string())));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn drives_from_dedupes_by_device_and_flags_optical() {
        let mounts = "/dev/mapper/luks-x / btrfs rw 0 0\n\
             /dev/mapper/luks-x /home btrfs rw 0 0\n\
             /dev/sr0 /run/media/u/DVD iso9660 ro 0 0\n\
             tmpfs /tmp tmpfs rw 0 0\n";
        let ds = drives_from(mounts, &|_| false, &|_| None, &|_| (100, 40));
        assert_eq!(ds.len(), 2);
        assert_eq!(ds[0].mount, PathBuf::from("/"));
        assert!(!ds[0].removable && !ds[0].optical);
        assert_eq!(ds[1].mount, PathBuf::from("/run/media/u/DVD"));
        assert!(ds[1].optical);
        assert!(ds[1].removable);
        assert_eq!(ds[1].total, 100);
        assert_eq!(ds[1].free, 40);
    }

    #[test]
    fn drives_from_asks_removable_by_full_device_basename() {
        let mounts = "/dev/sda2 / ext4 rw 0 0\n/dev/nvme0n1p2 /boot vfat rw 0 0\n";
        let asked = std::cell::RefCell::new(Vec::new());
        let sys_removable = |d: &str| {
            asked.borrow_mut().push(d.to_string());
            false
        };
        drives_from(mounts, &sys_removable, &|_| None, &|_| (0, 0));
        assert_eq!(asked.into_inner(), vec!["sda2", "nvme0n1p2"]);
    }

    #[test]
    fn drives_from_never_asks_removable_for_optical() {
        let mounts = "/dev/sr0 /run/media/u/DVD iso9660 ro 0 0\n";
        let asked = std::cell::RefCell::new(Vec::new());
        let sys_removable = |d: &str| {
            asked.borrow_mut().push(d.to_string());
            false
        };
        let ds = drives_from(mounts, &sys_removable, &|_| None, &|_| (0, 0));
        assert!(ds[0].removable);
        assert!(asked.into_inner().is_empty());
    }

    #[test]
    fn drives_from_excludes_loop_and_pseudo_file_systems() {
        let mounts = "/dev/loop0 /snap/core/1 squashfs ro 0 0\n\
             /dev/sda1 / ext4 rw 0 0\n";
        let ds = drives_from(mounts, &|_| false, &|_| None, &|_| (0, 0));
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].mount, PathBuf::from("/"));
    }

    /// A root that is not a block device (a ZFS pool, an NFS server, a
    /// container's `overlay`) still reaches Computer.
    #[test]
    fn drives_from_lists_a_root_that_is_not_a_block_device() {
        let mounts = "overlay / overlay rw 0 0\n\
             tmpfs /tmp tmpfs rw 0 0\n\
             zroot/home /home zfs rw 0 0\n";
        let ds = drives_from(mounts, &|_| false, &|_| None, &|_| (100, 40));
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].mount, PathBuf::from("/"));
        assert_eq!(ds[0].device, "overlay");
        assert_eq!(ds[0].fs, "overlay");
        assert!(!ds[0].removable && !ds[0].optical);
        assert_eq!((ds[0].total, ds[0].free), (100, 40));
    }

    /// `/proc/mounts` can name `/` twice (the initramfs `rootfs` line, then the
    /// real root): the list gets one root, a block device preferred to the placeholder.
    #[test]
    fn drives_from_lists_the_root_once_when_mounts_names_it_twice() {
        let ds = drives_from(
            "rootfs / rootfs rw 0 0\n/dev/sda1 / ext4 rw 0 0\n",
            &|_| false,
            &|_| None,
            &|_| (0, 0),
        );
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].device, "/dev/sda1");

        // With no block device to prefer, the line that replaced the placeholder counts.
        let ds = drives_from(
            "rootfs / rootfs rw 0 0\nzroot/ROOT/default / zfs rw 0 0\n",
            &|_| false,
            &|_| None,
            &|_| (0, 0),
        );
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].device, "zroot/ROOT/default");
    }

    #[test]
    fn drives_from_unescapes_octal_sequences_in_mount() {
        let mounts = "/dev/sdb1 /run/media/u/My\\040Disk vfat rw 0 0\n";
        let ds = drives_from(mounts, &|_| true, &|_| None, &|_| (0, 0));
        assert_eq!(ds[0].mount, PathBuf::from("/run/media/u/My Disk"));
    }

    #[test]
    fn unescape_mount_handles_all_four_escapes() {
        assert_eq!(unescape_mount(b"My\\040Disk"), b"My Disk");
        assert_eq!(unescape_mount(b"a\\011b"), b"a\tb");
        assert_eq!(unescape_mount(b"a\\012b"), b"a\nb");
        assert_eq!(unescape_mount(b"a\\134b"), b"a\\b");
        assert_eq!(unescape_mount(b"/dev/sda1"), b"/dev/sda1");
        assert_eq!(unescape_mount(b"a\\777b"), b"a\\777b");
        assert_eq!(unescape_mount(b"a\\377b"), b"a\xffb");
    }

    #[test]
    fn drives_from_keeps_a_mount_point_that_is_not_utf8() {
        let ds = drives_from(
            b"/dev/sdb1 /run/media/u/Caf\xe9 vfat rw 0 0\n",
            &|_| true,
            &|_| None,
            &|_| (0, 0),
        );
        let mount = PathBuf::from(OsString::from_vec(b"/run/media/u/Caf\xe9".to_vec()));
        assert_eq!(ds[0].mount, mount);
    }

    /// A table naming a non-UTF-8 path is still read, so the network mount in it stays out of the walk.
    #[test]
    fn mounted_drives_skip_a_network_mount_in_a_table_that_is_not_utf8() {
        let t = tempfile::tempdir().unwrap();
        let (root, table) = (t.path().join("media"), t.path().join("mounts"));
        std::fs::create_dir_all(root.join("NAS")).unwrap();
        std::fs::create_dir_all(root.join("STICK")).unwrap();
        let mut text = b"/dev/sdb1 /media/Caf\xe9 vfat rw 0 0\n".to_vec();
        text.extend_from_slice(
            format!("srv:/x {} nfs4 rw 0 0\n", root.join("NAS").display()).as_bytes(),
        );
        text.extend_from_slice(
            format!("/dev/sdc1 {} vfat rw 0 0\n", root.join("STICK").display()).as_bytes(),
        );
        std::fs::write(&table, text).unwrap();
        let mut drives = MountedDrives::at(table, vec![root.clone()]);
        assert_eq!(drives.current(), [root.join("STICK")]);
    }

    #[test]
    fn disk_name_strips_common_partition_suffixes() {
        assert_eq!(disk_name("sda2"), "sda");
        assert_eq!(disk_name("nvme0n1p2"), "nvme0n1");
        assert_eq!(disk_name("mmcblk0p1"), "mmcblk0");
    }

    #[test]
    fn dialog_places_run_recent_home_desktop_documents_downloads_computer() {
        let places = dialog_places(true);
        let labels: Vec<&str> = places.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(labels[0], "My Recent Documents");
        assert!(labels[1].ends_with("Home"), "the home tile is second");
        assert_eq!(
            labels[2..],
            ["Desktop", "Documents", "Downloads", "Computer"]
        );
        assert!(is_recent(places[0].dir.as_deref().unwrap()));
        assert_eq!(
            dialog_places(false)[0].dir,
            None,
            "recent_files = false greys it"
        );
        assert_eq!(
            dialog_parent(Path::new(RECENT)),
            None,
            "Up is greyed in Recent"
        );
        assert_eq!(
            places[5].dir.as_deref(),
            Some(Path::new("/")),
            "Computer is /"
        );
    }

    #[test]
    fn parse_user_dir_expands_home_and_quotes() {
        let home = Path::new("/home/u");
        let text =
            "# comment\nXDG_DOWNLOAD_DIR=\"$HOME/Downloads\"\nXDG_DESKTOP_DIR=\"$HOME/Asztal\"\n";
        assert_eq!(
            parse_user_dir(text, home, "DESKTOP"),
            Some(PathBuf::from("/home/u/Asztal"))
        );
        assert_eq!(
            parse_user_dir(text, home, "DOWNLOAD"),
            Some(PathBuf::from("/home/u/Downloads"))
        );
        assert_eq!(
            parse_user_dir("XDG_DESKTOP_DIR=\"/srv/desk\"\n", home, "DESKTOP"),
            Some(PathBuf::from("/srv/desk"))
        );
        assert_eq!(
            parse_user_dir("XDG_DOWNLOAD_DIR=\"$HOME/x\"\n", home, "DESKTOP"),
            None
        );
        assert_eq!(
            parse_user_dir("XDG_DESKTOP_DIR=\"desk\"\n", home, "DESKTOP"),
            None
        );
    }
}
