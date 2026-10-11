//! Desktop icons: entries from the desktop directory, the system icons,
//! names and clipboard rules. The grid, the cell drawing and the rename box

use reprisefs::apps::{self, AppEntry};
use reprisefs::{icons, names, places};
pub use reprisekit::controls::iconview::*;
use std::path::{Path, PathBuf};

pub const DRAG_PX: i32 = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Launch(AppEntry),
    Open(PathBuf),
    Uri(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    File,
    /// The Recycle Bin: opens the trash, takes drops, cannot be renamed,
    /// cut or deleted.
    RecycleBin,
    /// Documents, Computer, Home: open a place, cannot be cut; Delete only
    /// hides the icon and Rename changes its label.
    Place(Place),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Documents,
    Computer,
    Home,
}

impl Place {
    pub fn from_config(name: &str) -> Option<Place> {
        Some(match name {
            "documents" => Place::Documents,
            "computer" => Place::Computer,
            "home" => Place::Home,
            _ => return None,
        })
    }

    pub fn config_name(self) -> &'static str {
        match self {
            Place::Documents => "documents",
            Place::Computer => "computer",
            Place::Home => "home",
        }
    }

    pub fn label(self) -> String {
        match self {
            Place::Documents => "Documents".into(),
            Place::Computer => "Computer".into(),
            Place::Home => places::home_label(),
        }
    }
}

pub fn place_shown(
    name: &str,
    names: &std::collections::BTreeMap<String, String>,
) -> Option<DesktopEntry> {
    let mut entry = place(name)?;
    if let Some(label) = names.get(name) {
        entry.label = label.clone();
    }
    Some(entry)
}

pub fn rename_place(
    names: &mut std::collections::BTreeMap<String, String>,
    place: Place,
    typed: &str,
) -> bool {
    let typed = typed.trim();
    if typed.is_empty() {
        return false;
    }
    let key = place.config_name();
    let default = place.label();
    let current = names.get(key).unwrap_or(&default);
    if typed == current {
        return false;
    }
    if typed == default {
        names.remove(key);
    } else {
        names.insert(key.to_string(), typed.to_string());
    }
    true
}

pub fn rename_file(entry: &DesktopEntry, typed: &str) -> Result<Option<PathBuf>, String> {
    let typed = typed.trim();
    let (Some(dir), Some(old)) = (entry.path.parent(), entry.path.file_name()) else {
        return Ok(None);
    };
    if typed.is_empty() {
        return Ok(None);
    }
    let old = old.to_string_lossy();
    let wanted = format!("{typed}{}", entry.hidden_ext());
    match names::verdict(&old, &wanted, &[]) {
        names::Verdict::Cancel => Ok(None),
        names::Verdict::Refuse(text) => Err(text),
        names::Verdict::Rename(name) => {
            let to = dir.join(name);
            reprisefs::hidden::rename(&entry.path, &to)
                .map_err(|e| names::failed_text(&old, &e))?;
            Ok(Some(to))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopEntry {
    /// The file name on disk, lossy when it is not UTF-8 (a system icon's
    /// default label): the key of its grid position. Never `Name=`.
    pub name: String,
    pub label: String,
    pub icon: String,
    pub target: Target,
    pub path: PathBuf,
    pub shortcut: bool,
    pub kind: Kind,
}

impl DesktopEntry {
    /// The extension `label` hides (".txt"), or "" when nothing is hidden;
    /// a rename puts it back on the typed name.
    pub fn hidden_ext(&self) -> &str {
        self.name.strip_prefix(self.label.as_str()).unwrap_or("")
    }

    pub fn shortcut_base(&self) -> &str {
        if self.label_is_name_key() {
            &self.label
        } else {
            &self.name
        }
    }

    /// A launcher or a web link in the desktop folder: its label is the
    /// file's `Name=`, which a rename changes while the file keeps its name.
    pub fn label_is_name_key(&self) -> bool {
        match self.target {
            Target::Launch(_) => true,
            Target::Uri(_) => self.kind == Kind::File,
            Target::Open(_) => false,
        }
    }

    pub fn id(&self) -> IconId {
        match self.kind {
            Kind::File => IconId::File(self.path.clone()),
            kind => IconId::System(kind),
        }
    }
}

/// Which icon a menu or drag is aimed at. A rescan reorders `entries`, so
/// this is resolved to an index (`find`) only when the command runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IconId {
    File(PathBuf),
    System(Kind),
}

impl DesktopEntry {
    pub fn is(&self, id: &IconId) -> bool {
        match id {
            IconId::File(path) => self.kind == Kind::File && self.path == *path,
            IconId::System(kind) => self.kind == *kind,
        }
    }
}

pub fn find(entries: &[DesktopEntry], id: &IconId) -> Option<usize> {
    entries.iter().position(|e| e.is(id))
}

pub fn place(name: &str) -> Option<DesktopEntry> {
    let place = Place::from_config(name)?;
    let entry = |icon: &str, target: Target, path: PathBuf| DesktopEntry {
        name: place.label(),
        label: place.label(),
        icon: icon.into(),
        target,
        path,
        shortcut: false,
        kind: Kind::Place(place),
    };
    match place {
        Place::Documents => {
            let docs = places::user_dir("DOCUMENTS", "Documents")?;
            Some(entry("folder-documents", Target::Open(docs.clone()), docs))
        }
        Place::Computer => Some(entry(
            "computer",
            Target::Uri("computer:///".into()),
            PathBuf::from("/"),
        )),
        Place::Home => {
            let home = PathBuf::from(std::env::var_os("HOME")?);
            Some(entry("user-home", Target::Open(home.clone()), home))
        }
    }
}

/// The argument vector that opens a system icon's place, run with no
/// shell. `reprise-explorer` is the file manager, and it takes either a
/// directory or one of its special names ("Computer", "Recycle Bin"),
/// which is what the GVfs URIs on these entries stand for. It ships in the same package as the compositor, so there is
/// no fallback. None for a place whose path is not UTF-8, which no
/// `Exec=` line can hold.
pub fn place_argv(entry: &DesktopEntry) -> Option<Vec<String>> {
    let explorer = |place: &str| vec!["reprise-explorer".to_string(), place.to_string()];
    Some(match (&entry.target, entry.kind) {
        (Target::Launch(app), _) => reprisefs::apps::exec_argv(&app.exec_line),
        (_, Kind::RecycleBin) => explorer("Recycle Bin"),
        (_, Kind::Place(Place::Computer)) => explorer("Computer"),
        (Target::Open(path), _) => explorer(path.to_str()?),
        (Target::Uri(uri), _) => vec!["xdg-open".to_string(), uri.clone()],
    })
}

pub fn place_command(entry: &DesktopEntry) -> Option<String> {
    place_argv(entry).map(|argv| reprisefs::apps::command_line(&argv))
}

pub fn recycle_bin(files: PathBuf, empty: bool) -> DesktopEntry {
    DesktopEntry {
        name: "Recycle Bin".into(),
        label: "Recycle Bin".into(),
        icon: if empty {
            "user-trash".into()
        } else {
            "user-trash-full".into()
        },
        target: Target::Uri("trash:///".into()),
        path: files,
        shortcut: false,
        kind: Kind::RecycleBin,
    }
}

pub fn desktop_dir() -> Option<PathBuf> {
    places::user_dir("DESKTOP", "Desktop")
}

pub fn shell_place(kind: &Kind) -> Option<reprisefs::places::ShellPlace> {
    use reprisefs::places::ShellPlace;
    Some(match kind {
        Kind::File => return None,
        Kind::RecycleBin => ShellPlace::RecycleBin,
        Kind::Place(Place::Documents) => ShellPlace::Documents,
        Kind::Place(Place::Computer) => ShellPlace::Computer,
        Kind::Place(Place::Home) => ShellPlace::Home,
    })
}

pub fn document_tip(icon: &str) -> Option<&'static str> {
    Some(match icon {
        "folder-documents" => "Contains letters, reports, and other documents and files.",
        "folder-pictures" => "Contains digital photos, images, and graphic files.",
        "folder-music" => "Contains music and other audio files.",
        "folder-videos" => "Contains movies and other video files.",
        "folder-download" => "Contains files downloaded from the Internet.",
        _ => return None,
    })
}

/// Whether Allow Launching applies to `entry`: a file in the desktop
/// folder, shown as a plain file, that holds a displayable application.
/// Reads the file (a regular file of at most 64 KiB, opened without
/// blocking; see `apps::read_desktop_file`).
pub fn can_allow_launching(entry: &DesktopEntry) -> bool {
    entry.kind == Kind::File
        && matches!(entry.target, Target::Open(_))
        && entry.name.ends_with(".desktop")
        && apps::read_desktop_file(&entry.path)
            .is_ok_and(|text| apps::parse_desktop(&text, &entry.name).is_some())
}

pub fn scan(
    dir: &Path,
    globs: &reprisefs::apps::MimeDb,
    has_icon: &dyn Fn(&str) -> bool,
) -> Vec<DesktopEntry> {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::debug!("{}: {e}", dir.display());
            return Vec::new();
        }
        Err(e) => {
            log::warn!("{}: {e}", dir.display());
            return Vec::new();
        }
    };
    let listed = reprisefs::hidden::Listed::read(dir);
    let mut folders = Vec::new();
    let mut files = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        // `path` keeps a name that is not UTF-8; only `name` and the label
        // show it lossily.
        if listed.hides(&entry.file_name()) {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().into_owned();
        // The entry's own type comes from the listing (d_type); one stat
        // follows a symlink, and a dangling one shows as a plain file.
        let symlink = entry.file_type().is_ok_and(|t| t.is_symlink());
        let meta = std::fs::metadata(&path).ok();
        if meta.as_ref().is_some_and(|m| m.is_dir()) {
            let icon = reprisefs::meta::folder_icon(&path).unwrap_or_else(|| "folder".into());
            let group = if symlink { &mut files } else { &mut folders };
            group.push(DesktopEntry {
                label: file_name.clone(),
                name: file_name,
                icon,
                target: Target::Open(path.clone()),
                path,
                shortcut: symlink,
                kind: Kind::File,
            });
            continue;
        }
        if let Some(sc) = meta.as_ref().and_then(|m| apps::shortcut_at(&path, m)) {
            let (label, icon) = (sc.name().to_string(), sc.icon().to_string());
            files.push(DesktopEntry {
                name: file_name,
                label,
                icon,
                target: match sc {
                    apps::Shortcut::App(app) => Target::Launch(app),
                    apps::Shortcut::Link(link) => Target::Uri(link.url),
                },
                path,
                shortcut: true,
                kind: Kind::File,
            });
            continue;
        }
        let mime = globs.mime_of_name(&file_name);
        let icon = icons::pick_icon(mime.as_deref(), &path, has_icon);
        files.push(DesktopEntry {
            label: names::label_for(&file_name, globs),
            name: file_name,
            icon,
            target: Target::Open(path.clone()),
            path,
            shortcut: symlink,
            kind: Kind::File,
        });
    }
    // Ties (two launchers with one Name=, names differing only in case)
    // break on the file name: read_dir's order is the filesystem's hash
    // order and differs from one machine to the next.
    folders.sort_by_cached_key(|e| (e.name.to_lowercase(), e.name.clone()));
    files.sort_by_cached_key(|e| (e.shortcut_base().to_lowercase(), e.name.clone()));
    folders.extend(files);
    folders
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shell_places_have_a_tip_and_a_file_none() {
        use reprisefs::places::ShellPlace;
        assert_eq!(shell_place(&Kind::File), None);
        assert_eq!(shell_place(&Kind::RecycleBin), Some(ShellPlace::RecycleBin));
        for (p, want) in [
            (Place::Documents, ShellPlace::Documents),
            (Place::Computer, ShellPlace::Computer),
            (Place::Home, ShellPlace::Home),
        ] {
            assert_eq!(shell_place(&Kind::Place(p)), Some(want));
        }
        if !reprisekit::text::test_tahoma() {
            return;
        }
        let theme = reprisekit::theme::Theme::default();
        let text = reprisekit::text::TextRenderer::load(&theme.fonts);
        for p in [
            ShellPlace::Documents,
            ShellPlace::Computer,
            ShellPlace::Home,
            ShellPlace::RecycleBin,
        ] {
            let lines = [p.tip().to_string()];
            let (w, h) = reprisekit::controls::infotip::size(&text, &theme, &lines);
            assert_eq!(h, 25, "{p:?}: one 13 px line in 5 px of padding");
            assert!(w <= 330, "{p:?}: {w} px wide");
        }
    }

    fn fixture(name: &str) -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("reprise-desktop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn place_command_for_a_folder_that_is_not_utf8_is_none() {
        use std::os::unix::ffi::OsStrExt;
        let docs = |path: PathBuf| DesktopEntry {
            name: "Documents".into(),
            label: "Documents".into(),
            icon: "folder-documents".into(),
            target: Target::Open(path.clone()),
            path,
            shortcut: false,
            kind: Kind::Place(Place::Documents),
        };
        assert_eq!(
            place_command(&docs(PathBuf::from("/home/u/Docs"))).as_deref(),
            Some("reprise-explorer /home/u/Docs")
        );
        let odd = PathBuf::from(std::ffi::OsStr::from_bytes(b"/home/u/Docs\xff"));
        assert_eq!(place_command(&docs(odd)), None);
    }

    #[test]
    fn a_shortcut_to_a_place_opens_that_place() {
        use reprisefs::apps;
        let argv = |e: &DesktopEntry| {
            let exec = place_command(e).unwrap();
            let text = apps::launcher_text("s", None, apps::Launch::Exec(&exec)).unwrap();
            let app = apps::parse_desktop(&text, "s.desktop").unwrap();
            apps::launch_argv(&app, "foot")
        };
        let bin = recycle_bin(PathBuf::from("/t/files"), false);
        assert_eq!(argv(&bin), ["reprise-explorer", "Recycle Bin"]);
        let pc = place("computer").unwrap();
        assert_eq!(argv(&pc), ["reprise-explorer", "Computer"]);
        let docs = DesktopEntry {
            target: Target::Open(PathBuf::from("/home/u/it's mine")),
            kind: Kind::Place(Place::Documents),
            ..pc
        };
        assert_eq!(argv(&docs), ["reprise-explorer", "/home/u/it's mine"]);
    }

    fn file_entry(path: PathBuf, label: &str) -> DesktopEntry {
        DesktopEntry {
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            label: label.into(),
            icon: "text-x-generic".into(),
            target: Target::Open(path.clone()),
            path,
            shortcut: false,
            kind: Kind::File,
        }
    }

    #[test]
    fn rename_file_keeps_the_hidden_extension_and_stays_in_the_folder() {
        let dir = fixture("rename-ok");
        let _c = Cleanup(dir.clone());
        std::fs::write(dir.join("notes.txt"), "a").unwrap();
        let entry = file_entry(dir.join("notes.txt"), "notes");
        assert_eq!(rename_file(&entry, "  notes "), Ok(None));
        assert_eq!(rename_file(&entry, ""), Ok(None));
        assert_eq!(
            rename_file(&entry, "report"),
            Ok(Some(dir.join("report.txt")))
        );
        assert_eq!(std::fs::read(dir.join("report.txt")).unwrap(), b"a");
    }

    #[test]
    fn rename_file_refuses_a_path_or_a_dot_entry_and_moves_nothing() {
        let dir = fixture("rename-path");
        let _c = Cleanup(dir.clone());
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a"), "a").unwrap();
        let entry = file_entry(dir.join("a"), "a");
        let Err(text) = rename_file(&entry, "sub/x") else {
            panic!("sub/x should be refused");
        };
        assert!(text.starts_with("A file name cannot contain"), "{text}");
        for bad in [".", ".."] {
            assert_eq!(
                rename_file(&entry, bad),
                Err(
                    "Cannot rename a: The file name you specified is not valid or too long.".into()
                ),
                "{bad}"
            );
        }
        assert!(dir.join("a").is_file(), "a stays where it was");
        assert!(!dir.join("sub/x").exists(), "nothing lands in sub");
    }

    #[test]
    fn rename_file_refuses_a_taken_name_even_a_dangling_link() {
        let dir = fixture("rename-taken");
        let _c = Cleanup(dir.clone());
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        std::fs::write(dir.join("b.txt"), "b").unwrap();
        std::os::unix::fs::symlink(dir.join("gone"), dir.join("c.txt")).unwrap();
        let entry = file_entry(dir.join("a.txt"), "a");
        let taken = "Cannot rename a.txt: A file with the name you specified already exists. \
                     Specify a different file name.";
        assert_eq!(rename_file(&entry, "b"), Err(taken.into()));
        assert_eq!(rename_file(&entry, "c"), Err(taken.into()));
        assert_eq!(std::fs::read(dir.join("b.txt")).unwrap(), b"b");
        assert!(dir.join("c.txt").is_symlink(), "the link is still there");
        assert!(dir.join("a.txt").is_file());
    }

    #[test]
    fn place_icons_rename_through_the_state() {
        use std::collections::BTreeMap;
        let mut names = BTreeMap::new();
        let pc = place_shown("computer", &names).unwrap();
        assert_eq!(
            (pc.name.as_str(), pc.label.as_str()),
            ("Computer", "Computer")
        );
        assert!(rename_place(&mut names, Place::Computer, "  Box  "));
        assert_eq!(names.get("computer").map(String::as_str), Some("Box"));
        let pc = place_shown("computer", &names).unwrap();
        assert_eq!((pc.name.as_str(), pc.label.as_str()), ("Computer", "Box"));
        assert!(!rename_place(&mut names, Place::Computer, "Box"));
        assert!(!rename_place(&mut names, Place::Computer, ""));
        assert!(rename_place(&mut names, Place::Computer, "Computer"));
        assert!(names.is_empty());
        assert!(!rename_place(&mut names, Place::Computer, "Computer"));
        assert_eq!(Place::from_config("documents"), Some(Place::Documents));
        assert_eq!(Place::Documents.config_name(), "documents");
        assert_eq!(Place::from_config("my-documents"), None, "mapped at parse");
        assert!(place_shown("my-music", &names).is_none());
    }

    #[test]
    fn home_is_the_home_folder_opened_by_its_path() {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return;
        };
        let e = place("home").unwrap();
        assert_eq!(e.label, places::home_label());
        assert!(e.label.ends_with("Home"));
        assert_eq!(e.icon, "user-home");
        assert_eq!(e.kind, Kind::Place(Place::Home));
        assert_eq!(e.target, Target::Open(home.clone()));
        assert_eq!(e.path, home);
        assert_eq!(
            place_argv(&e),
            home.to_str()
                .map(|h| vec!["reprise-explorer".to_string(), h.to_string()])
        );
        let docs = place("documents").unwrap();
        assert_eq!(docs.label, "Documents");
        assert_eq!(
            docs.target,
            Target::Open(places::user_dir("DOCUMENTS", "Documents").unwrap())
        );
    }

    #[test]
    fn scan_classifies_folders_desktop_files_and_others() {
        let dir = fixture("scan");
        let _cleanup = Cleanup(dir.clone());
        std::fs::create_dir(dir.join("Photos")).unwrap();
        std::fs::write(
            dir.join("firefox.desktop"),
            "[Desktop Entry]\nType=Application\nName=Firefox\nExec=firefox %u\nIcon=firefox\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("noicon.desktop"),
            "[Desktop Entry]\nType=Application\nName=Bare\nExec=bare\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("broken.desktop"),
            "[Desktop Entry]\nType=Link\nName=x\n",
        )
        .unwrap();
        std::fs::write(dir.join("notes.txt"), "hi").unwrap();
        std::fs::write(dir.join(".hidden"), "listed.txt\n").unwrap();
        std::fs::write(dir.join("listed.txt"), "hi").unwrap();
        for f in ["firefox.desktop", "noicon.desktop", "broken.desktop"] {
            apps::allow_launching(&dir.join(f)).unwrap();
        }
        let list = scan(&dir, &apps::mime_globs(), &|_| false);
        let names: Vec<&str> = list.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "Photos",
                "noicon.desktop",
                "broken.desktop",
                "firefox.desktop",
                "notes.txt"
            ]
        );
        assert_eq!(list[0].icon, "folder");
        assert_eq!(list[0].target, Target::Open(dir.join("Photos")));
        assert_eq!(list[1].icon, "application-x-executable");
        assert_eq!(list[2].target, Target::Open(dir.join("broken.desktop")));
        assert_eq!(list[3].icon, "firefox");
        assert!(
            matches!(&list[3].target, Target::Launch(a) if reprisefs::apps::exec_argv(&a.exec_line) == ["firefox"])
        );
        assert_eq!(list[4].icon, "text-x-generic");
        assert_eq!(list[4].target, Target::Open(dir.join("notes.txt")));
    }

    #[test]
    fn scan_labels_files_without_known_extensions_and_keeps_the_ext_for_renames() {
        let dir = fixture("scan_label");
        let _cleanup = Cleanup(dir.clone());
        std::fs::write(dir.join("notes.txt"), "").unwrap();
        std::fs::write(dir.join("odd.zzzq"), "").unwrap();
        std::fs::create_dir(dir.join("Photos.txt")).unwrap();
        let globs = reprisefs::apps::MimeDb::parse(["50:text/plain:*.txt\n"]);
        let has = |n: &str| n == "gnome-mime-text-plain";
        let list = scan(&dir, &globs, &has);
        let by = |n: &str| list.iter().find(|e| e.name == n).unwrap();
        assert_eq!(by("notes.txt").label, "notes");
        assert_eq!(by("notes.txt").icon, "gnome-mime-text-plain");
        assert_eq!(by("odd.zzzq").icon, "text-x-generic");
        assert_eq!(by("notes.txt").hidden_ext(), ".txt");
        assert_eq!(by("odd.zzzq").label, "odd.zzzq");
        assert_eq!(by("odd.zzzq").hidden_ext(), "");
        assert_eq!(by("Photos.txt").label, "Photos.txt");
    }

    #[test]
    fn an_icon_id_follows_its_file_across_a_rescan() {
        let dir = fixture("iconid");
        let _c = Cleanup(dir.clone());
        std::fs::write(dir.join("b.txt"), "").unwrap();
        let with_bin = |mut files: Vec<DesktopEntry>| {
            files.insert(0, recycle_bin(dir.join("trash"), true));
            files
        };
        let before = with_bin(scan(&dir, &apps::mime_globs(), &|_| false));
        let aimed = before[1].id();
        let bin = before[0].id();
        assert_eq!(aimed, IconId::File(dir.join("b.txt")));
        std::fs::write(dir.join("a.txt"), "").unwrap();
        let after = with_bin(scan(&dir, &apps::mime_globs(), &|_| false));
        assert_eq!(find(&after, &aimed), Some(2), "b.txt is row 2");
        assert_eq!(find(&after, &bin), Some(0), "the Recycle Bin is row 0");
        std::fs::remove_file(dir.join("b.txt")).unwrap();
        let gone = with_bin(scan(&dir, &apps::mime_globs(), &|_| false));
        assert_eq!(find(&gone, &aimed), None);
    }

    #[test]
    fn launchers_sharing_a_name_key_on_their_file_names() {
        let dir = fixture("samename");
        let _c = Cleanup(dir.clone());
        let text = "[Desktop Entry]\nType=Application\nName=Firefox\nExec=firefox\n";
        for f in ["firefox.desktop", "firefox (2).desktop"] {
            std::fs::write(dir.join(f), text).unwrap();
            apps::allow_launching(&dir.join(f)).unwrap();
        }
        let list = scan(&dir, &apps::mime_globs(), &|_| false);
        let keys: Vec<(&str, &str)> = list
            .iter()
            .map(|e| (e.name.as_str(), e.label.as_str()))
            .collect();
        assert_eq!(
            keys,
            [
                ("firefox (2).desktop", "Firefox"),
                ("firefox.desktop", "Firefox")
            ]
        );
    }

    #[test]
    fn scan_of_missing_dir_is_empty() {
        assert!(
            scan(
                Path::new("/nonexistent/reprise-desktop"),
                &apps::MimeDb::empty(),
                &|_| false
            )
            .is_empty()
        );
    }

    #[test]
    fn scan_marks_launchers_and_symlinks_as_shortcuts_sorted_among_the_files() {
        let dir = std::env::temp_dir().join(format!("reprise-shortcut-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Reports")).unwrap();
        std::fs::write(dir.join("notes.txt"), "").unwrap();
        std::fs::write(
            dir.join("app.desktop"),
            "[Desktop Entry]\nType=Application\nName=App\nExec=app\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(dir.join("notes.txt"), dir.join("link.txt")).unwrap();
        std::os::unix::fs::symlink(dir.join("Reports"), dir.join("Reports link")).unwrap();
        apps::allow_launching(&dir.join("app.desktop")).unwrap();
        let list = scan(&dir, &apps::mime_globs(), &|_| false);
        let flag = |n: &str| list.iter().find(|e| e.name == n).unwrap().shortcut;
        assert!(!flag("Reports"));
        assert!(!flag("notes.txt"));
        assert!(flag("app.desktop"));
        assert!(flag("link.txt"));
        assert!(flag("Reports link"));
        let names: Vec<&str> = list.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Reports",
                "app.desktop",
                "link.txt",
                "notes.txt",
                "Reports link"
            ],
            "a symlink to a folder sorts among the files"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_shows_a_file_whose_name_is_not_utf8_under_its_own_path() {
        use std::os::unix::ffi::OsStrExt;
        let dir = fixture("not_utf8");
        let _cleanup = Cleanup(dir.clone());
        let path = dir.join(std::ffi::OsStr::from_bytes(b"caf\xe9.txt"));
        std::fs::write(&path, "").unwrap();
        let list = scan(
            &dir,
            &apps::MimeDb::parse(["50:text/plain:*.txt\n"]),
            &|_| false,
        );
        assert_eq!(list.len(), 1, "the file is listed");
        assert_eq!(list[0].path, path);
        assert_eq!(list[0].target, Target::Open(path.clone()));
        assert_eq!(list[0].name, "caf\u{fffd}.txt");
        assert_eq!(list[0].label, "caf\u{fffd}");
    }

    fn chmod_exec(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A .desktop is a launcher only when executable and trusted; without
    /// either it is a plain file, shown under its file name and opened by
    /// xdg-open, never run.
    #[test]
    fn a_desktop_file_is_a_launcher_only_when_executable_and_trusted() {
        use std::os::unix::fs::PermissionsExt;
        let dir = fixture("untrusted");
        let _cleanup = Cleanup(dir.clone());
        let text = "[Desktop Entry]\nType=Application\nName=Document\nIcon=text-x-generic\nExec=rm -rf ~\n";
        let path = dir.join("document.desktop");
        std::fs::write(&path, text).unwrap();
        let plain = |list: &[DesktopEntry]| {
            list[0].name == "document.desktop"
                && matches!(list[0].target, Target::Open(_))
                && !list[0].shortcut
        };
        assert!(
            plain(&scan(&dir, &apps::mime_globs(), &|_| false)),
            "not executable: a file"
        );
        chmod_exec(&path);
        assert!(!apps::is_trusted(&path));
        let list = scan(&dir, &apps::mime_globs(), &|_| false);
        assert!(plain(&list), "untrusted: a file");
        assert!(can_allow_launching(&list[0]));
        apps::allow_launching(&path).unwrap();
        let list = scan(&dir, &apps::mime_globs(), &|_| false);
        assert_eq!(list[0].label, "Document");
        assert!(
            matches!(list[0].target, Target::Launch(_)),
            "trusted: a launcher"
        );
        assert!(!can_allow_launching(&list[0]), "already a launcher");
        // A .desktop that is no application never gets the row: trusting
        // it would not make it a launcher.
        std::fs::write(&path, "[Desktop Entry]\nType=Link\nName=x\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let list = scan(&dir, &apps::mime_globs(), &|_| false);
        assert!(!can_allow_launching(&list[0]));
    }

    #[test]
    fn a_trusted_desktop_file_that_is_no_application_shows_as_a_file() {
        let dir = fixture("nonapp");
        let _cleanup = Cleanup(dir.clone());
        for (file, text) in [
            ("link.desktop", "[Desktop Entry]\nType=Link\nName=Web\n"),
            (
                "web.desktop",
                "[Desktop Entry]\nType=Link\nName=Example\nURL=https://example.org\n",
            ),
            (
                "hidden.desktop",
                "[Desktop Entry]\nType=Application\nName=Hidden\nExec=true\nNoDisplay=true\n",
            ),
        ] {
            std::fs::write(dir.join(file), text).unwrap();
            apps::allow_launching(&dir.join(file)).unwrap();
        }
        let list = scan(&dir, &apps::mime_globs(), &|_| false);
        let names: Vec<&str> = list.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["web.desktop", "hidden.desktop", "link.desktop"]);
        assert!(
            list[1..]
                .iter()
                .all(|e| matches!(e.target, Target::Open(_)) && !e.shortcut),
            "both open with xdg-open, neither is a shortcut"
        );
        let web = &list[0];
        assert_eq!(web.label, "Example");
        assert_eq!(web.icon, "text-html");
        assert!(web.shortcut && web.label_is_name_key());
        assert_eq!(
            place_argv(web),
            Some(vec![
                "xdg-open".to_string(),
                "https://example.org".to_string()
            ])
        );
    }

    /// An executable FIFO named `*.desktop` would block `open(2)` forever,
    /// and a launcher past 64 KiB is not read at all: both show as plain
    /// files.
    #[test]
    fn scan_shows_a_fifo_and_an_oversized_launcher_as_plain_files() {
        let dir = fixture("fifo");
        let _cleanup = Cleanup(dir.clone());
        let fifo = dir.join("pipe.desktop");
        let c = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: `c` is a NUL-terminated path that outlives the call.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o755) }, 0);
        chmod_exec(&fifo);
        let big = format!(
            "[Desktop Entry]\nType=Application\nName=Big\nExec=big\n#{}\n",
            "x".repeat(64 * 1024)
        );
        std::fs::write(dir.join("big.desktop"), big).unwrap();
        apps::allow_launching(&dir.join("big.desktop")).unwrap();
        let list = scan(&dir, &apps::mime_globs(), &|_| false);
        let names: Vec<&str> = list.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["big.desktop", "pipe.desktop"]);
        assert!(
            list.iter().all(|e| matches!(e.target, Target::Open(_))),
            "both open with xdg-open"
        );
    }
}
