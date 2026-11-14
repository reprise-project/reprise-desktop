//! What uninstalls a program: the Flatpak installation that exports its desktop file, the
//! system's packages through PackageKit, or deleting a launcher an app installed.

use std::path::{Path, PathBuf};

/// A Flatpak installation Reprise installs into and removes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Installation {
    System,
    User,
}

impl Installation {
    /// The `flatpak` option that picks it.
    pub fn flag(self) -> &'static str {
        match self {
            Installation::System => "--system",
            Installation::User => "--user",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remover {
    /// A Flatpak app exported by this installation.
    Flatpak(Installation),
    /// A desktop file in a system data directory, which an installed package may own.
    Package,
    /// A launcher an app installed through the DynamicLauncher portal ([`remove_launcher`]).
    Launcher,
    /// The user's own entry, or one from another Flatpak installation.
    None,
}

/// Where xdg-desktop-portal keeps the launchers and icons apps install, under the user's data
/// directory. Each launcher is linked into the user's `applications` folder.
const PORTAL_APPS: &str = "xdg-desktop-portal/applications";
const PORTAL_ICONS: &str = "xdg-desktop-portal/icons";

fn data_home() -> Option<PathBuf> {
    crate::apps::user_applications_dir().and_then(|d| d.parent().map(Into::into))
}

/// What removes the program whose desktop file is `file`, from where the file is.
pub fn remover_of(file: &Path) -> Remover {
    remover_among(file, &crate::flatpak::installations())
}

/// [`remover_of`] with [`crate::flatpak::installations`] read once by the caller, for many files.
pub fn remover_among(file: &Path, installs: &[PathBuf]) -> Remover {
    let target = std::fs::read_link(file)
        .ok()
        .map(|t| lexical(&file.parent().unwrap_or(Path::new("/")).join(t)));
    remover_in(file, target.as_deref(), installs, data_home().as_deref())
}

/// [`remover_of`] for `file` linking to `target`, with `installs` as
/// [`crate::flatpak::installations`] lists them (the system installation, the user's, then any
/// others) and the user's data directory.
fn remover_in(
    file: &Path,
    target: Option<&Path>,
    installs: &[PathBuf],
    data_home: Option<&Path>,
) -> Remover {
    let exports = |i: &PathBuf| i.join("exports");
    if let Some(i) = installs.iter().position(|i| file.starts_with(exports(i))) {
        return match i {
            0 => Remover::Flatpak(Installation::System),
            1 => Remover::Flatpak(Installation::User),
            _ => Remover::None,
        };
    }
    if let Some(h) = data_home {
        let launchers = h.join(PORTAL_APPS);
        if file.starts_with(&launchers) || target.is_some_and(|t| t.starts_with(&launchers)) {
            return Remover::Launcher;
        }
        if file.starts_with(h) {
            return Remover::None;
        }
    }
    Remover::Package
}

/// `path` with its `.` and `..` resolved without reading the file system.
fn lexical(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// Delete the portal-installed launcher whose desktop file is `file` (its link in the user's
/// `applications` folder or the entry itself): the link, the entry and the icon the portal saved
/// for it, as xdg-desktop-portal's own Uninstall does.
pub fn remove_launcher(file: &Path) -> std::io::Result<()> {
    let home = data_home().ok_or_else(|| std::io::Error::other("no home directory"))?;
    remove_launcher_in(file, &home)
}

fn remove_launcher_in(file: &Path, data_home: &Path) -> std::io::Result<()> {
    let id = file
        .file_name()
        .ok_or_else(|| std::io::Error::other("no file name"))?;
    let entry = data_home.join(PORTAL_APPS).join(id);
    let link = data_home.join("applications").join(id);
    let gone = |r: std::io::Result<()>| match r {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        r => r,
    };
    if link.symlink_metadata().is_ok_and(|m| m.is_symlink()) {
        gone(std::fs::remove_file(&link))?;
    }
    let icon = crate::apps::read_desktop_file(&entry)
        .ok()
        .and_then(|text| {
            let kv = crate::apps::desktop_entry_kv(&text);
            kv.get("Icon").map(|v| PathBuf::from(v.as_ref()))
        });
    gone(std::fs::remove_file(&entry))?;
    match icon.map(|i| lexical(&i)) {
        Some(i) if i.starts_with(data_home.join(PORTAL_ICONS)) => gone(std::fs::remove_file(i)),
        _ => Ok(()),
    }
}

/// The id of the Flatpak app that exports desktop file `file` with contents `text`: the
/// `X-Flatpak` key Flatpak adds on export, else the file's name.
pub fn flatpak_app(text: &str, file: &Path) -> String {
    let kv = crate::apps::desktop_entry_kv(text);
    match kv.get("X-Flatpak") {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

/// A `.flatpakref` file: the app or runtime it installs and the repository it comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlatpakRef {
    pub id: String,
    /// Its `Title`, else the id.
    pub title: String,
    pub branch: String,
    /// The repository's address.
    pub url: String,
    /// `RuntimeRepo`: the `.flatpakrepo` the runtime comes from, empty when it names none.
    pub runtime_repo: String,
}

/// The `[Flatpak Ref]` group of `text`; None without a `Name`.
pub fn flatpak_ref(text: &str) -> Option<FlatpakRef> {
    let kv = crate::apps::group_kv(text, "[Flatpak Ref]");
    let get = |k: &str| kv.get(k).map(|v| v.trim().to_string()).unwrap_or_default();
    let id = get("Name");
    if id.is_empty() {
        return None;
    }
    let title = Some(get("Title")).filter(|t| !t.is_empty());
    Some(FlatpakRef {
        title: title.unwrap_or_else(|| id.clone()),
        branch: get("Branch"),
        url: get("Url"),
        runtime_repo: get("RuntimeRepo"),
        id,
    })
}

/// Whether the system bus can start PackageKit: its service file is in a directory the bus
/// reads.
pub fn packagekit_present() -> bool {
    [
        "/usr/local/share/dbus-1/system-services",
        "/usr/share/dbus-1/system-services",
        "/lib/dbus-1/system-services",
    ]
    .iter()
    .any(|d| {
        Path::new(d)
            .join("org.freedesktop.PackageKit.service")
            .is_file()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installs() -> Vec<PathBuf> {
        vec![
            "/var/lib/flatpak".into(),
            "/home/u/.local/share/flatpak".into(),
            "/opt/extra".into(),
        ]
    }

    #[test]
    fn an_exported_file_names_its_installation() {
        let r = |p: &str| {
            remover_in(
                Path::new(p),
                None,
                &installs(),
                Some(Path::new("/home/u/.local/share")),
            )
        };
        assert_eq!(
            r("/var/lib/flatpak/exports/share/applications/org.example.App.desktop"),
            Remover::Flatpak(Installation::System)
        );
        assert_eq!(
            r("/home/u/.local/share/flatpak/exports/share/applications/org.example.App.desktop"),
            Remover::Flatpak(Installation::User)
        );
        assert_eq!(
            r("/opt/extra/exports/share/applications/org.example.App.desktop"),
            Remover::None,
            "another installation"
        );
    }

    #[test]
    fn a_system_file_may_be_a_package_and_the_users_own_is_not() {
        let home = Some(Path::new("/home/u/.local/share"));
        let r = |p: &str| remover_in(Path::new(p), None, &installs(), home);
        assert_eq!(r("/usr/share/applications/foot.desktop"), Remover::Package);
        assert_eq!(
            r("/usr/local/share/applications/x.desktop"),
            Remover::Package
        );
        assert_eq!(
            r("/home/u/.local/share/applications/mine.desktop"),
            Remover::None
        );
    }

    #[test]
    fn a_portal_launcher_is_its_link_or_entry() {
        let home = Some(Path::new("/home/u/.local/share"));
        let link = Path::new("/home/u/.local/share/applications/org.example.Web.app.desktop");
        let target = lexical(Path::new(
            "/home/u/.local/share/applications/../xdg-desktop-portal/applications/org.example.Web.app.desktop",
        ));
        let r = |f: &Path, t: Option<&Path>| remover_in(f, t, &installs(), home);
        assert_eq!(r(link, Some(&target)), Remover::Launcher);
        assert_eq!(r(&target, None), Remover::Launcher);
        let elsewhere = Path::new("/home/u/notes/mine.desktop");
        assert_eq!(
            r(link, Some(elsewhere)),
            Remover::None,
            "a link of the user's own"
        );
    }

    #[test]
    fn removing_a_launcher_deletes_its_link_entry_and_saved_icon() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let id = "org.example.Web.app.desktop";
        let apps = home.join("applications");
        let entries = home.join(PORTAL_APPS);
        let icons = home.join(PORTAL_ICONS).join("128x128");
        for d in [&apps, &entries, &icons] {
            std::fs::create_dir_all(d).unwrap();
        }
        let icon = icons.join("org.example.Web.app.png");
        std::fs::write(&icon, b"png").unwrap();
        let entry = entries.join(id);
        let text = format!("[Desktop Entry]\nName=Web\nIcon={}\n", icon.display());
        std::fs::write(&entry, text).unwrap();
        let link = apps.join(id);
        std::os::unix::fs::symlink(format!("../{PORTAL_APPS}/{id}"), &link).unwrap();
        let theirs = home.join("theirs.png");
        std::fs::write(&theirs, b"png").unwrap();

        remove_launcher_in(&link, home).unwrap();
        assert!(link.symlink_metadata().is_err(), "link removed");
        assert!(!entry.exists(), "entry removed");
        assert!(!icon.exists(), "saved icon removed");

        std::fs::write(
            &entry,
            format!("[Desktop Entry]\nIcon={}\n", theirs.display()),
        )
        .unwrap();
        remove_launcher_in(&entry, home).unwrap();
        assert!(theirs.exists(), "an icon outside the portal's folder stays");
        assert!(remove_launcher_in(&entry, home).is_ok(), "already gone");
    }

    #[test]
    fn a_flatpakref_names_its_app_branch_and_repository() {
        let text = "[Flatpak Ref]\nName=org.example.Pinball\nBranch=stable\n\
                    Title=Pinball\nUrl=https://dl.example.org/repo/\nIsRuntime=false\n\
                    RuntimeRepo=https://dl.flathub.org/repo/flathub.flatpakrepo\n";
        assert_eq!(
            flatpak_ref(text),
            Some(FlatpakRef {
                id: "org.example.Pinball".into(),
                title: "Pinball".into(),
                branch: "stable".into(),
                url: "https://dl.example.org/repo/".into(),
                runtime_repo: "https://dl.flathub.org/repo/flathub.flatpakrepo".into(),
            })
        );
        let bare = flatpak_ref("[Flatpak Ref]\nName=org.example.Bare\n").unwrap();
        assert_eq!(bare.title, "org.example.Bare");
        assert_eq!(bare.runtime_repo, "");
        assert_eq!(flatpak_ref("[Flatpak Ref]\nTitle=Nameless\n"), None);
    }

    #[test]
    fn the_flatpak_key_names_the_app_over_the_file() {
        let file =
            Path::new("/var/lib/flatpak/exports/share/applications/org.example.App.Tool.desktop");
        let text = "[Desktop Entry]\nName=Tool\nX-Flatpak=org.example.App\n";
        assert_eq!(flatpak_app(text, file), "org.example.App");
        assert_eq!(
            flatpak_app("[Desktop Entry]\nName=Tool\n", file),
            "org.example.App.Tool"
        );
    }
}
