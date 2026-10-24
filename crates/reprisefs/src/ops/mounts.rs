use std::io;
use std::path::{Path, PathBuf};

/// Whether a directory on device `dev`, inside a directory on
/// `parent_dev`, is the root of another file system: a mount point, or a
/// btrfs subvolume. A move off its device copies it, empties it, then
/// fails to remove the root itself, so none may be a move's source.
pub fn mount_root(dev: u64, parent_dev: u64) -> bool {
    dev != parent_dev
}

/// [`mount_root`] for `path` against its lexical parent. `/` is one; a
/// symlink never is (it lives in its parent's file system). A path that
/// cannot be read is not: the operation on it fails on its own.
pub fn is_mount_point(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if meta.is_symlink() {
        return false;
    }
    let parent = match path.parent() {
        None => return true,
        Some(p) if p.as_os_str().is_empty() => Path::new("."),
        Some(p) => p,
    };
    std::fs::metadata(parent).is_ok_and(|p| mount_root(meta.dev(), p.dev()))
}

/// `Err` when `path` [is a mount point](is_mount_point): the check every
/// move, trash and erase makes before it touches the source.
pub fn refuse_mount_point(path: &Path) -> io::Result<()> {
    if is_mount_point(path) {
        return Err(io::Error::new(
            io::ErrorKind::ResourceBusy,
            format!("{} is the root of a mounted drive", path.display()),
        ));
    }
    Ok(())
}

/// `Err` when a folder anywhere under `root` is a [`mount_root`]: the
/// check before a tree is copied then removed, erased or trashed, since
/// each would empty the drive mounted there. Links are not followed. A
/// file or a link as `root` holds nothing.
pub fn refuse_holding_mount(root: &Path) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let dev_of = |p: &Path| std::fs::symlink_metadata(p).map(|m| m.dev());
    if let Some(inner) = mount_under(root, &dev_of)? {
        return Err(io::Error::new(
            io::ErrorKind::ResourceBusy,
            format!(
                "{} contains a mounted drive at {}",
                root.display(),
                inner.display()
            ),
        ));
    }
    Ok(())
}

/// The first folder under `root` whose device, per `dev_of`, differs from
/// the folder holding it. A folder below `root` that cannot be listed or
/// looked at is skipped: the copy and the remove cannot reach into it
/// either, and fail there on their own.
pub(super) fn mount_under(
    root: &Path,
    dev_of: &dyn Fn(&Path) -> io::Result<u64>,
) -> io::Result<Option<PathBuf>> {
    if !std::fs::symlink_metadata(root)?.is_dir() {
        return Ok(None);
    }
    let mut dirs = vec![(root.to_path_buf(), dev_of(root)?)];
    while let Some((dir, dev)) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            // `DirEntry::file_type` does not follow a link.
            if !entry.file_type().is_ok_and(|k| k.is_dir()) {
                continue;
            }
            let path = entry.path();
            let Ok(sub) = dev_of(&path) else {
                continue;
            };
            if mount_root(sub, dev) {
                return Ok(Some(path));
            }
            dirs.push((path, sub));
        }
    }
    Ok(None)
}
