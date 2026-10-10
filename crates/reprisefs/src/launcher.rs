use crate::apps::{desktop_entry_kv, escape_value, exec_argv, label_value, read_desktop_file};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Exec,
    Path,
    Comment,
    Icon,
    Url,
    Link,
}

pub const FIELDS: [Key; 3] = [Key::Exec, Key::Path, Key::Comment];

impl Key {
    pub fn name(self) -> &'static str {
        match self {
            Key::Exec => "Exec",
            Key::Path => "Path",
            Key::Comment => "Comment",
            Key::Icon => "Icon",
            Key::Url => "URL",
            Key::Link => "",
        }
    }

    pub fn field(self) -> Option<usize> {
        match self {
            Key::Exec | Key::Url | Key::Link => Some(0),
            Key::Path => Some(1),
            Key::Comment => Some(2),
            Key::Icon => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Program,
    Symlink,
    Web,
}

/// String escapes decoded, but a program's `target` keeps its field codes
/// (`%u`, `%%`) so the Target field writes back the line the file holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Launcher {
    pub kind: Kind,
    /// A symlink's target as the link holds it: relative to its folder, or absolute.
    pub target: String,
    pub path: String,
    pub comment: String,
    pub icon: Option<String>,
    pub target_type: String,
    pub location: String,
    pub runs: bool,
}

impl Launcher {
    pub fn parse(text: &str) -> Option<Launcher> {
        let kv = desktop_entry_kv(text);
        let get = |k: &str| kv.get(k).map(|v| v.to_string()).unwrap_or_default();
        if get("Type") == "Link" {
            let link = crate::apps::parse_link(text)?;
            return Some(Launcher {
                kind: Kind::Web,
                target: link.url,
                icon: link.icon,
                ..Launcher::default()
            });
        }
        // A compositor action button is an Application entry with no `Exec`.
        if get("Type") != "Application" || get("Exec").trim().is_empty() {
            return None;
        }
        Some(Launcher {
            kind: Kind::Program,
            target: get("Exec"),
            path: get("Path"),
            comment: get("Comment"),
            icon: Some(get("Icon")).filter(|i| !i.is_empty()),
            target_type: "Application".to_string(),
            location: String::new(),
            runs: false,
        })
    }

    /// The symlink at `link`, its target typed against `db`; None when
    /// `link` is no symlink. A target that is not there types as a file
    /// named so would.
    pub fn symlink(link: &Path, db: &crate::apps::MimeDb) -> Option<Launcher> {
        use std::os::unix::fs::PermissionsExt;
        let target = std::fs::read_link(link).ok()?;
        let to = resolve(link, &target);
        let meta = std::fs::metadata(&to).ok();
        let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());
        let mime = (!is_dir)
            .then(|| crate::apps::mime_of_in(db, &to))
            .flatten();
        Some(Launcher {
            kind: Kind::Symlink,
            target: target.to_string_lossy().into_owned(),
            target_type: crate::listing::kind_text_in(db, &to, is_dir, mime.as_deref()),
            location: folder_name(&to),
            runs: meta.is_some_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0),
            ..Launcher::default()
        })
    }

    /// The values of the Shortcut tab's fields, in [`FIELDS`] order;
    /// Start in and Comment are "" for a symlink and a web link.
    pub fn fields(&self) -> [String; 3] {
        [self.target.clone(), self.path.clone(), self.comment.clone()]
    }

    pub fn read(path: &Path) -> Option<Launcher> {
        let mut l = Launcher::parse(&read_desktop_file(path).ok()?)?;
        l.location = l.program().map(|p| folder_name(&p)).unwrap_or_default();
        Some(l)
    }

    pub fn at(path: &Path, db: &crate::apps::MimeDb) -> Option<Launcher> {
        let listed =
            std::fs::metadata(path).is_ok_and(|m| crate::apps::shortcut_at(path, &m).is_some());
        listed
            .then(|| Launcher::read(path))
            .flatten()
            .or_else(|| Launcher::symlink(path, db))
    }

    pub fn program(&self) -> Option<PathBuf> {
        match self.kind {
            Kind::Program => program(&self.target),
            _ => None,
        }
    }

    pub fn folder(&self, link: &Path) -> Option<PathBuf> {
        let to = match self.kind {
            Kind::Program => self.program()?,
            Kind::Symlink => resolve(link, Path::new(&self.target)),
            Kind::Web => return None,
        };
        to.parent().map(Path::to_path_buf)
    }

    pub fn find(&self, field: &str, link: &Path) -> Option<PathBuf> {
        match self.kind {
            Kind::Program => program(field),
            Kind::Symlink => Some(resolve(link, Path::new(field.trim())))
                .filter(|to| !field.trim().is_empty() && std::fs::symlink_metadata(to).is_ok()),
            Kind::Web => None,
        }
    }

    pub fn not_found_text(&self, field: &str) -> String {
        match self.kind {
            Kind::Program => format!("Reprise cannot find the program '{}'.", field.trim()),
            _ => format!("Reprise cannot find '{}'.", field.trim()),
        }
    }

    pub fn edits(
        &self,
        moves: &[(usize, String)],
        icon: Option<Option<String>>,
    ) -> Vec<(Key, String)> {
        let key = |k: usize| match (self.kind, k) {
            (Kind::Program, k) => FIELDS.get(k).copied(),
            (Kind::Symlink, 0) => Some(Key::Link),
            (Kind::Web, 0) => Some(Key::Url),
            _ => None,
        };
        let mut out: Vec<(Key, String)> = moves
            .iter()
            .filter_map(|(k, v)| Some((key(*k)?, v.clone())))
            .collect();
        if let Some(icon) = icon.filter(|_| self.kind != Kind::Symlink) {
            out.push((Key::Icon, icon.unwrap_or_default()));
        }
        out
    }

    pub fn set(&mut self, key: Key, value: &str) {
        let value = value.to_string();
        match key {
            Key::Exec | Key::Url | Key::Link => self.target = value,
            Key::Path => self.path = value,
            Key::Comment => self.comment = value,
            Key::Icon => self.icon = Some(value).filter(|v| !v.is_empty()),
        }
    }
}

fn resolve(link: &Path, target: &Path) -> PathBuf {
    let to = match link.parent() {
        Some(dir) if target.is_relative() => dir.join(target),
        _ => target.to_path_buf(),
    };
    match (to.parent(), to.file_name()) {
        (Some(dir), Some(name)) => std::fs::canonicalize(dir)
            .map(|d| d.join(name))
            .unwrap_or(to),
        _ => to,
    }
}

pub fn program(exec: &str) -> Option<PathBuf> {
    let first = exec_argv(exec).into_iter().next()?;
    if first.contains('/') {
        return Some(PathBuf::from(first));
    }
    crate::search::which(&first)
}

pub fn folder_name(path: &Path) -> String {
    match path.parent() {
        Some(dir) => match dir.file_name() {
            Some(n) => n.to_string_lossy().into_owned(),
            None => dir.to_string_lossy().into_owned(),
        },
        None => String::new(),
    }
}

/// Edits `text`'s `[Desktop Entry]` group; every other line stays byte for
/// byte. An empty `Exec` or `URL` edit is dropped: a launcher without one opens
/// nothing.
pub fn with_keys<S: AsRef<str>>(text: &str, edits: &[(Key, S)]) -> String {
    let edits: Vec<(Key, &str)> = edits
        .iter()
        .map(|(k, v)| (*k, v.as_ref()))
        .filter(|(k, v)| match k {
            Key::Exec | Key::Url => !v.trim().is_empty(),
            Key::Link => false,
            _ => true,
        })
        .collect();
    let line_for = |k: Key, v: &str| {
        let value = match k {
            // `%` stays as typed: the field shows the field codes.
            Key::Exec | Key::Url => escape_value(&v.replace(char::is_control, "")),
            _ => label_value(v),
        };
        format!("{}={value}", k.name())
    };
    let mut out = String::with_capacity(text.len() + 64);
    let mut done = vec![false; edits.len()];
    let mut in_entry = false;
    let mut had_entry = false;
    // Where the group's last entry line ends: missing keys go there, ahead of
    // blank lines and comments before the next group.
    let mut entry_end = None;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let trimmed = body.trim();
        if trimmed.starts_with('[') {
            in_entry = trimmed == "[Desktop Entry]";
            had_entry |= in_entry;
            out.push_str(line);
            if in_entry {
                entry_end = Some(out.len());
            }
            continue;
        }
        if in_entry && !trimmed.is_empty() && !trimmed.starts_with('#') {
            let key = trimmed.split_once('=').map(|(k, _)| k.trim());
            if let Some(j) = edits.iter().position(|(k, _)| Some(k.name()) == key) {
                let (k, v) = edits[j];
                if !done[j] && !v.is_empty() {
                    out.push_str(&line_for(k, v));
                    out.push_str(&line[body.len()..]);
                    entry_end = Some(out.len());
                }
                done[j] = true;
                continue;
            }
            out.push_str(line);
            entry_end = Some(out.len());
            continue;
        }
        out.push_str(line);
    }
    let mut added = String::new();
    for (j, &(k, v)) in edits.iter().enumerate() {
        if !done[j] && !v.is_empty() {
            added.push_str(&line_for(k, v));
            added.push('\n');
        }
    }
    if added.is_empty() {
        return out;
    }
    match entry_end.filter(|_| had_entry) {
        Some(at) => {
            // The last entry line may lack its newline at the end of the file.
            let at = if at == out.len() && !out.ends_with('\n') {
                out.push('\n');
                out.len()
            } else {
                at
            };
            out.insert_str(at, &added);
        }
        None => {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("[Desktop Entry]\n");
            out.push_str(&added);
        }
    }
    out
}

/// Replaces the file in one rename, keeping its mode and trust marker; writes
/// nothing when nothing changes.
pub fn write_keys<S: AsRef<str>>(path: &Path, edits: &[(Key, S)]) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    if let Some((_, to)) = edits.iter().find(|(k, _)| *k == Key::Link) {
        return relink(path, to.as_ref());
    }
    if edits
        .iter()
        .any(|(k, v)| *k == Key::Url && !crate::dropdata::is_web_url(v.as_ref().trim()))
    {
        return Err(Error::new(ErrorKind::InvalidInput, "not a web address"));
    }
    let old = read_desktop_file(path)?;
    let new = with_keys(&old, edits);
    if new == old {
        return Ok(());
    }
    crate::apps::rewrite_launcher(path, &new)
}

pub fn relink(link: &Path, to: &str) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    let to = to.trim();
    if to.is_empty() || to.contains(char::is_control) {
        return Err(Error::new(ErrorKind::InvalidInput, "no target"));
    }
    if !std::fs::symlink_metadata(link)?.file_type().is_symlink() {
        return Err(Error::new(ErrorKind::InvalidInput, "not a symlink"));
    }
    if std::fs::read_link(link)?.as_os_str() == to {
        return Ok(());
    }
    let name = link
        .file_name()
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "no file name"))?
        .to_string_lossy();
    for n in 0..100 {
        let tmp = link.with_file_name(format!(".{name}.relink{n}"));
        match std::os::unix::fs::symlink(to, &tmp) {
            Ok(()) => {
                return std::fs::rename(&tmp, link).inspect_err(|_| {
                    let _ = std::fs::remove_file(&tmp);
                });
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(Error::new(
        ErrorKind::AlreadyExists,
        "no free name beside it",
    ))
}

pub fn icon_names(apps: &[crate::apps::AppEntry], has: impl Fn(&str) -> bool) -> Vec<String> {
    let mut sorted: Vec<&crate::apps::AppEntry> = apps.iter().collect();
    sorted.sort_by_key(|a| a.name.to_lowercase());
    let mut out: Vec<String> = Vec::new();
    for icon in sorted.into_iter().filter_map(|a| a.icon.as_deref()) {
        if !icon.contains('/') && !out.iter().any(|n| n == icon) && has(icon) {
            out.push(icon.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIREFOX: &str = "\
# a comment
[Desktop Entry]
Type=Application
Name=Firefox
Name[de]=Feuerfuchs
Comment=Browse the web
Comment[de]=Im Internet surfen
Exec=firefox %u
Icon=firefox
X-Custom=kept

[Desktop Action new-window]
Name=New Window
Exec=firefox --new-window %u
";

    #[test]
    fn a_launcher_reads_its_own_keys() {
        let l = Launcher::parse(FIREFOX).expect("a launcher");
        assert_eq!(l.target, "firefox %u");
        assert_eq!(l.path, "");
        assert_eq!(l.comment, "Browse the web");
        assert_eq!(l.icon.as_deref(), Some("firefox"));
        let spaced = Launcher::parse("[Desktop Entry]\nType=Application\nExec=a\\sb\nPath=/t\n")
            .expect("a launcher");
        assert_eq!(
            (spaced.target.as_str(), spaced.path.as_str()),
            ("a b", "/t")
        );
        assert_eq!(
            Launcher::parse("[Desktop Entry]\nType=Link\nName=Ex\nURL=https://example.org\n"),
            Some(Launcher {
                kind: Kind::Web,
                target: "https://example.org".into(),
                ..Launcher::default()
            })
        );
        assert_eq!(
            Launcher::parse("[Desktop Entry]\nType=Link\nName=Ex\nURL=file:///etc\n"),
            None,
            "a link to no web page"
        );
        assert_eq!(
            Launcher::parse("[Desktop Entry]\nType=Application\nX-Reprise-Action=desktop\n"),
            None
        );
    }

    /// An edit changes only its own unlocalized line in the entry group.
    #[test]
    fn edits_keep_every_other_line() {
        let out = with_keys(
            FIREFOX,
            &[
                (Key::Exec, "firefox --private-window %u"),
                (Key::Path, "/home/u/Downloads"),
                (Key::Comment, ""),
                (Key::Icon, "web-browser"),
            ],
        );
        assert_eq!(
            out,
            "\
# a comment
[Desktop Entry]
Type=Application
Name=Firefox
Name[de]=Feuerfuchs
Comment[de]=Im Internet surfen
Exec=firefox --private-window %u
Icon=web-browser
X-Custom=kept
Path=/home/u/Downloads

[Desktop Action new-window]
Name=New Window
Exec=firefox --new-window %u
"
        );
        let back = Launcher::parse(&out).expect("still a launcher");
        assert_eq!(back.target, "firefox --private-window %u");
        assert_eq!(back.path, "/home/u/Downloads");
        assert_eq!(back.comment, "");
        assert_eq!(with_keys(FIREFOX, &[(Key::Exec, "  ")]), FIREFOX);
        assert_eq!(with_keys::<&str>(FIREFOX, &[]), FIREFOX);
    }

    #[test]
    fn values_round_trip_through_the_string_escapes() {
        let typed = "C:\\new\nline";
        let out = with_keys(
            "[Desktop Entry]\nType=Application\nExec=a",
            &[(Key::Comment, typed)],
        );
        assert_eq!(
            out,
            "[Desktop Entry]\nType=Application\nExec=a\nComment=C:\\\\newline\n"
        );
        assert_eq!(Launcher::parse(&out).unwrap().comment, "C:\\newline");
        assert_eq!(
            with_keys("# nothing\n", &[(Key::Path, "/tmp")]),
            "# nothing\n[Desktop Entry]\nPath=/tmp\n"
        );
    }

    #[test]
    fn a_write_replaces_the_launcher_and_keeps_its_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("firefox.desktop");
        std::fs::write(&path, FIREFOX).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o750)).unwrap();
        write_keys(&path, &[(Key::Path, "/srv")]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(Launcher::parse(&text).unwrap().path, "/srv");
        assert!(text.contains("Comment[de]=Im Internet surfen\n"));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o750);
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        write_keys(&path, &[(Key::Path, "/srv")]).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            before
        );
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "no temp file is left beside it"
        );
    }

    #[test]
    fn the_target_is_the_program_exec_starts() {
        let l = Launcher {
            target: "/usr/lib/firefox/firefox %u".into(),
            ..Launcher::default()
        };
        assert_eq!(l.program(), Some(PathBuf::from("/usr/lib/firefox/firefox")));
        assert_eq!(folder_name(&l.program().unwrap()), "firefox");
        assert_eq!(program(""), None);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sh.desktop");
        crate::apps::write_launcher(
            &path,
            "[Desktop Entry]\nType=Application\nName=Shell\nExec=/bin/sh -l\n",
        )
        .unwrap();
        assert_eq!(Launcher::read(&path).unwrap().location, "bin");
        assert_eq!(
            Launcher::at(&path, &crate::apps::MimeDb::empty())
                .unwrap()
                .location,
            "bin"
        );
    }

    #[test]
    fn the_moves_become_edits() {
        let moves = [(0, "a %u".to_string()), (2, String::new())];
        let l = Launcher::default();
        assert_eq!(
            l.edits(&moves, Some(Some("web-browser".into()))),
            [
                (Key::Exec, "a %u".to_string()),
                (Key::Comment, String::new()),
                (Key::Icon, "web-browser".to_string())
            ]
        );
        assert_eq!(l.edits(&[], Some(None)), [(Key::Icon, String::new())]);
        assert_eq!(l.edits(&[], None), []);
    }

    #[test]
    fn a_symlink_is_a_shortcut_to_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let docs = dir.path().join("docs");
        std::fs::create_dir(&docs).unwrap();
        let link = dir.path().join("to-docs");
        std::os::unix::fs::symlink("docs", &link).unwrap();
        let db = crate::apps::MimeDb::empty();
        let l = Launcher::symlink(&link, &db).expect("a symlink");
        assert_eq!((l.kind, l.target.as_str()), (Kind::Symlink, "docs"));
        assert_eq!(l.target_type, "File Folder");
        assert!(!l.runs);
        let real = std::fs::canonicalize(dir.path()).unwrap();
        assert_eq!(l.location, real.file_name().unwrap().to_string_lossy());
        assert_eq!(l.find("docs", &link), Some(real.join("docs")));
        assert_eq!(l.find("gone", &link), None);
        assert_eq!(l.find(" ", &link), None);
        assert_eq!(l.not_found_text(" gone "), "Reprise cannot find 'gone'.");
        assert_eq!(
            l.edits(&[(0, "/tmp".into())], Some(Some("folder".into()))),
            [(Key::Link, "/tmp".to_string())]
        );
        assert_eq!(
            Launcher::symlink(&docs, &db),
            None,
            "a folder is no symlink"
        );

        use std::os::unix::fs::PermissionsExt;
        let tool = dir.path().join("tool");
        std::fs::write(&tool, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let run = dir.path().join("run");
        std::os::unix::fs::symlink(&tool, &run).unwrap();
        assert!(
            Launcher::symlink(&run, &db).unwrap().runs,
            "a link to a program"
        );
        assert_eq!(folder_name(Path::new("/etc")), "/");
    }

    #[test]
    fn the_folder_is_the_full_path_of_the_target_location() {
        let dir = tempfile::tempdir().unwrap();
        let real = std::fs::canonicalize(dir.path()).unwrap();
        let link = real.join("to-docs");
        std::os::unix::fs::symlink("docs", &link).unwrap();
        let db = crate::apps::MimeDb::empty();
        let l = Launcher::symlink(&link, &db).unwrap();
        assert_eq!(l.folder(&link), Some(real.clone()));
        let etc = real.join("to-etc");
        std::os::unix::fs::symlink("/etc", &etc).unwrap();
        let l = Launcher::symlink(&etc, &db).unwrap();
        assert_eq!(l.folder(&etc), Some(PathBuf::from("/")));
        let prog = Launcher {
            target: "/usr/lib/firefox/firefox %u".into(),
            ..Launcher::default()
        };
        assert_eq!(prog.folder(&link), Some(PathBuf::from("/usr/lib/firefox")));
        let web =
            Launcher::parse("[Desktop Entry]\nType=Link\nName=Example\nURL=https://example.org/\n")
                .unwrap();
        assert_eq!(web.folder(&link), None);
    }

    #[test]
    fn a_symlink_is_relinked_in_one_rename() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("l");
        std::os::unix::fs::symlink("a", &link).unwrap();
        write_keys(&link, &[(Key::Link, " /srv ")]).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), PathBuf::from("/srv"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        assert!(relink(&link, "").is_err());
        let file = dir.path().join("f");
        std::fs::write(&file, "x").unwrap();
        assert!(relink(&file, "/srv").is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "x");
    }

    #[test]
    fn a_web_links_url_is_written_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ex.desktop");
        let text = "[Desktop Entry]\nType=Link\nName=Ex\nIcon=text-html\nURL=https://example.org\n";
        std::fs::write(&path, text).unwrap();
        let l = Launcher::read(&path).unwrap();
        let edits = l.edits(&[(0, "https://example.com/a b".into())], None);
        assert_eq!(edits, [(Key::Url, "https://example.com/a b".to_string())]);
        assert!(
            write_keys(&path, &edits).is_err(),
            "a space is no web address"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        write_keys(&path, &[(Key::Url, "https://example.com/")]).unwrap();
        let back = Launcher::read(&path).unwrap();
        assert_eq!(back.target, "https://example.com/");
        assert_eq!(back.icon.as_deref(), Some("text-html"));
        let mut l = l;
        l.set(Key::Url, "https://example.com/");
        assert_eq!(l, back);
    }
}
