//! What uninstalls a program: the Flatpak installation that exports its desktop file, or the
//! system's packages through PackageKit.

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
    /// The user's own entry, or one from another Flatpak installation.
    None,
}

/// What removes the program whose desktop file is `file`, from where the file is.
pub fn remover_of(file: &Path) -> Remover {
    let installs = crate::flatpak::installations();
    let data_home: Option<PathBuf> =
        crate::apps::user_applications_dir().and_then(|d| d.parent().map(Into::into));
    remover_in(file, &installs, data_home.as_deref())
}

/// [`remover_of`] with `installs` as [`crate::flatpak::installations`] lists them (the system
/// installation, the user's, then any others) and the user's data directory.
fn remover_in(file: &Path, installs: &[PathBuf], data_home: Option<&Path>) -> Remover {
    let exports = |i: &PathBuf| i.join("exports");
    if let Some(i) = installs.iter().position(|i| file.starts_with(exports(i))) {
        return match i {
            0 => Remover::Flatpak(Installation::System),
            1 => Remover::Flatpak(Installation::User),
            _ => Remover::None,
        };
    }
    if data_home.is_some_and(|h| file.starts_with(h)) {
        return Remover::None;
    }
    Remover::Package
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
        let r = |p: &str| remover_in(Path::new(p), &installs(), home);
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
    fn a_flatpakref_names_its_app_branch_and_repository() {
        let text = "[Flatpak Ref]\nName=org.example.Pinball\nBranch=stable\n\
                    Title=Pinball\nUrl=https://dl.example.org/repo/\nIsRuntime=false\n";
        assert_eq!(
            flatpak_ref(text),
            Some(FlatpakRef {
                id: "org.example.Pinball".into(),
                title: "Pinball".into(),
                branch: "stable".into(),
                url: "https://dl.example.org/repo/".into(),
            })
        );
        let bare = flatpak_ref("[Flatpak Ref]\nName=org.example.Bare\n").unwrap();
        assert_eq!(bare.title, "org.example.Bare");
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
