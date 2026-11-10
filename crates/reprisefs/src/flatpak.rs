//! Which Flatpak app a process belongs to, from the `.flatpak-info` key file Flatpak puts at the
//! root of every sandbox, and which apps the installations hold.

/// The app id of the Flatpak sandbox `pid` runs in, or None outside one. A claim any process in
/// its own mount namespace can make; a grant reads [`reprisebus::peer::scoped_flatpak_app`].
pub fn app_of_pid(pid: u32) -> Option<String> {
    app_of_info(&std::fs::read_to_string(format!("/proc/{pid}/root/.flatpak-info")).ok()?)
}

pub use reprisebus::peer::app_of_flatpak_info as app_of_info;

use std::path::{Path, PathBuf};

/// The Flatpak installations: the system one, the user's, and those `/etc/flatpak/installations.d`
/// adds.
pub fn installations() -> Vec<PathBuf> {
    let mut out = vec![PathBuf::from("/var/lib/flatpak")];
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    out.extend(data_home.map(|d| d.join("flatpak")));
    if let Ok(dir) = std::fs::read_dir("/etc/flatpak/installations.d") {
        let mut confs: Vec<PathBuf> = dir.flatten().map(|e| e.path()).collect();
        confs.sort();
        for conf in confs {
            out.extend(
                std::fs::read_to_string(conf)
                    .iter()
                    .flat_map(|t| added_paths(t)),
            );
        }
    }
    out
}

/// The `Path` of each `[Installation "..."]` group in an `installations.d` file.
fn added_paths(conf: &str) -> Vec<PathBuf> {
    let mut group = "";
    let mut out = Vec::new();
    for line in conf.lines().map(str::trim) {
        if let Some(g) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            group = g;
        } else if group.starts_with("Installation ")
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == "Path"
        {
            out.push(PathBuf::from(v.trim()));
        }
    }
    out
}

/// `key` in `[group]` of key file `text`.
fn value<'a>(text: &'a str, group: &str, key: &str) -> Option<&'a str> {
    let mut at = "";
    for line in text.lines().map(str::trim) {
        if let Some(g) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            at = g;
        } else if at == group
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == key
        {
            return Some(v.trim());
        }
    }
    None
}

/// The apps installed under `installations` whose sandbox may open a Wayland connection, by id,
/// sorted and each once.
pub fn wayland_apps(installations: &[PathBuf]) -> Vec<String> {
    let mut out: Vec<String> = installations
        .iter()
        .filter_map(|i| std::fs::read_dir(i.join("app")).ok())
        .flat_map(|d| d.flatten())
        .filter_map(|e| wayland_app(&e.path()))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The app installed in `dir` (`<installation>/app/<id>`) when its metadata names it by that id
/// and gives it a Wayland socket.
fn wayland_app(dir: &Path) -> Option<String> {
    let id = dir.file_name()?.to_str().filter(|id| valid_app_id(id))?;
    let text = std::fs::read_to_string(dir.join("current/active/metadata")).ok()?;
    let sockets = value(&text, "Context", "sockets").unwrap_or("");
    let wayland = sockets.split(';').any(|s| s.trim() == "wayland");
    (wayland && value(&text, "Application", "name") == Some(id)).then(|| id.to_string())
}

/// Flatpak's app id rules: at most 255 bytes, three or more dot-separated elements of ASCII
/// letters, digits, `_` and `-`, none empty or starting with a digit.
fn valid_app_id(id: &str) -> bool {
    let elements = id.split('.');
    id.len() <= 255
        && elements.clone().count() >= 3
        && elements.into_iter().all(|e| {
            !e.is_empty()
                && !e.starts_with(|c: char| c.is_ascii_digit())
                && e.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_is_named_in_the_application_group() {
        let info =
            "[Application]\nname=org.example.Sync\nruntime=runtime/x\n\n[Instance]\nname=other\n";
        assert_eq!(app_of_info(info).as_deref(), Some("org.example.Sync"));
        assert_eq!(app_of_info("[Instance]\nname=x\n"), None);
    }

    fn install(root: &Path, id: &str, metadata: &str) {
        let dir = root.join("app").join(id).join("x86_64/stable/active");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("metadata"), metadata).unwrap();
        let current = root.join("app").join(id).join("current");
        std::os::unix::fs::symlink("x86_64/stable", current).unwrap();
    }

    #[test]
    fn the_apps_with_a_wayland_socket_are_listed_from_every_installation() {
        let system = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        let app = |id: &str, sockets: &str| {
            format!("[Application]\nname={id}\nruntime=r\n\n[Context]\nsockets={sockets}\n")
        };
        install(
            system.path(),
            "org.example.Pass",
            &app("org.example.Pass", "x11;wayland;"),
        );
        install(
            system.path(),
            "org.example.Old",
            &app("org.example.Old", "x11;"),
        );
        install(
            user.path(),
            "org.example.Notes",
            &app("org.example.Notes", "wayland"),
        );
        install(
            user.path(),
            "org.example.Pass",
            &app("org.example.Pass", "wayland;"),
        );
        install(
            system.path(),
            "org.example.Bare",
            "[Application]\nname=org.example.Bare\n",
        );
        let missing = system.path().join("no-such-installation");
        assert_eq!(
            wayland_apps(&[system.path().into(), user.path().into(), missing]),
            ["org.example.Notes", "org.example.Pass"]
        );
    }

    #[test]
    fn an_app_is_listed_only_under_the_valid_id_its_metadata_names() {
        let root = tempfile::tempdir().unwrap();
        let app =
            |name: &str| format!("[Application]\nname={name}\n\n[Context]\nsockets=wayland;\n");
        install(root.path(), "org.example.Fake", &app("org.example.Pass"));
        install(root.path(), "example", &app("example"));
        install(root.path(), "org.2example.App", &app("org.2example.App"));
        install(root.path(), "org.example.Notes", &app("org.example.Notes"));
        assert_eq!(wayland_apps(&[root.path().into()]), ["org.example.Notes"]);
    }

    #[test]
    fn an_installations_d_file_adds_each_installation_path() {
        let conf = "[Installation \"extra\"]\nPath=/run/media/extra/flatpak\nDisplayName=Extra\n\n[Other]\nPath=/nope\n";
        assert_eq!(
            added_paths(conf),
            [PathBuf::from("/run/media/extra/flatpak")]
        );
    }

    #[test]
    fn a_process_outside_a_sandbox_has_no_app() {
        assert_eq!(app_of_pid(std::process::id()), None);
    }
}
