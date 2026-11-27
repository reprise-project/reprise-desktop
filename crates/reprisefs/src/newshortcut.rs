//! New > Shortcut's targets. A program from the list, an existing file or folder, or a command
//! line becomes a `Type=Application` launcher (a file opening through `xdg-open`, a folder in
//! Explorer, as Create Shortcut's launchers do); a web address becomes a `Type=Link`. The file is
//! written trusted, as the desktop runs only trusted launchers.

use reprisekit::tr;
use std::io;
use std::path::{Path, PathBuf};

use crate::apps::{self, AppEntry, Launch};

/// What a new shortcut opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    App(Box<AppEntry>),
    Path {
        path: PathBuf,
        is_dir: bool,
    },
    Web(String),
    /// A command line whose program was found.
    Command(String),
}

/// The location field's `text` read: a web address, a path (`~` standing for `home`) that
/// exists, or a command line whose program is on `PATH` or at its path. Err is the error box's
/// text.
pub fn read_typed(text: &str, home: Option<&Path>) -> Result<Target, String> {
    read_typed_with(text, home, |p| crate::autostart::find_program(p).is_some())
}

fn read_typed_with(
    text: &str,
    home: Option<&Path>,
    found: impl Fn(&str) -> bool,
) -> Result<Target, String> {
    let text = text.trim();
    if crate::dropdata::is_web_url(text) {
        return Ok(Target::Web(text.to_string()));
    }
    let path = match (text.strip_prefix('~'), home) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => {
            home.join(rest.trim_start_matches('/'))
        }
        _ => PathBuf::from(text),
    };
    if path.is_absolute()
        && let Ok(meta) = std::fs::metadata(&path)
    {
        return Ok(Target::Path {
            is_dir: meta.is_dir(),
            path,
        });
    }
    let argv = apps::exec_argv(text);
    match argv.first() {
        Some(program) if found(program) => Ok(Target::Command(text.to_string())),
        Some(program) => Err(tr!("shortcut-no-program", program = program.as_str())),
        None => Err(tr!("shortcut-no-location")),
    }
}

impl Target {
    /// The name the name page offers: the program's, the file's, the command's program, or the
    /// web address's host; never holding a `/`.
    pub fn default_name(&self) -> String {
        let file_name = |p: &Path| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string())
        };
        let name = match self {
            Target::App(app) => app.name.clone(),
            Target::Path { path, .. } => file_name(path),
            Target::Web(url) => web_host(url).unwrap_or(url).to_string(),
            Target::Command(cmd) => apps::exec_argv(cmd)
                .first()
                .map(|p| file_name(Path::new(p)))
                .unwrap_or_default(),
        };
        name.replace('/', "-")
    }

    /// The launcher's text, shown as `name`; None for a path or command a desktop entry cannot
    /// hold (a control character, a path that is not UTF-8).
    pub fn launcher(&self, name: &str) -> Option<String> {
        let (icon, command) = match self {
            Target::Web(url) => return apps::link_text(name, url),
            Target::App(app) => (
                app.icon.clone(),
                apps::command_line(&apps::exec_argv(&app.exec_line)),
            ),
            Target::Path { path, is_dir } => (
                Some(if *is_dir {
                    "folder".to_string()
                } else {
                    crate::icons::icon_for_file(path).to_string()
                }),
                apps::open_command_as(path, *is_dir)?,
            ),
            Target::Command(cmd) => (Some("application-x-executable".into()), cmd.clone()),
        };
        apps::launcher_text(name, icon.as_deref(), Launch::Exec(&command))
    }
}

/// `url`'s host, past any user part and without the port; None when it names none.
fn web_host(url: &str) -> Option<&str> {
    let (_, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = match host.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => host,
    };
    Some(host).filter(|h| !h.is_empty())
}

/// Writes launcher `text` into `dir` as `name.desktop`, numbered past a name taken, trusted
/// ([`apps::write_launcher`]).
pub fn write(dir: &Path, name: &str, text: &str) -> io::Result<PathBuf> {
    let file = format!("{name}.desktop");
    let mut last = None;
    for candidate in crate::names::numbered(file.as_ref(), crate::names::paren).take(1000) {
        let path = dir.join(candidate);
        match apps::write_launcher(&path, text) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.expect("the loop ran"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Result<Target, String> {
        read_typed_with(text, Some(Path::new("/home/u")), |p| p == "htop")
    }

    #[test]
    fn a_typed_location_reads_as_a_web_address_a_path_or_a_found_command() {
        assert_eq!(
            read(" https://example.org/a "),
            Ok(Target::Web("https://example.org/a".into()))
        );
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("notes.txt");
        std::fs::write(&file, "").unwrap();
        assert_eq!(
            read(&file.to_string_lossy()),
            Ok(Target::Path {
                path: file.clone(),
                is_dir: false
            })
        );
        assert_eq!(
            read(&tmp.path().to_string_lossy()),
            Ok(Target::Path {
                path: tmp.path().to_path_buf(),
                is_dir: true
            })
        );
        assert_eq!(read("htop -d 5"), Ok(Target::Command("htop -d 5".into())));
        let missing = read("htpo").unwrap_err();
        assert!(missing.contains("'htpo'"), "{missing}");
        assert!(read("~/gone.txt").is_err(), "a path that is not there");
    }

    #[test]
    fn a_tilde_stands_for_home() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "").unwrap();
        let got = read_typed_with("~/a.txt", Some(tmp.path()), |_| false);
        assert_eq!(
            got,
            Ok(Target::Path {
                path: tmp.path().join("a.txt"),
                is_dir: false
            })
        );
    }

    #[test]
    fn each_target_writes_the_launcher_the_desktop_opens() {
        let folder = Target::Path {
            path: "/home/u/Music".into(),
            is_dir: true,
        };
        assert_eq!(folder.default_name(), "Music");
        let text = folder.launcher("Music").unwrap();
        assert!(text.starts_with("[Desktop Entry]\nType=Application\nName=Music\n"));
        assert!(
            text.contains("Icon=folder\n")
                && text.contains("Exec=reprise-explorer /home/u/Music\n")
        );
        let file = Target::Path {
            path: "/home/u/a b.txt".into(),
            is_dir: false,
        };
        assert!(
            file.launcher("a")
                .unwrap()
                .contains("Exec=xdg-open \"/home/u/a b.txt\"\n")
        );
        let cmd = Target::Command("date +%s".into());
        assert_eq!(cmd.default_name(), "date");
        assert!(
            cmd.launcher("Date").unwrap().contains("Exec=date +%%s\n"),
            "a % is no field code"
        );
        let web = Target::Web("https://example.org".into());
        assert!(web.launcher("Example").unwrap().contains("Type=Link\n"));
        let app = apps::parse_desktop(
            "[Desktop Entry]\nType=Application\nName=Notepad\nIcon=notepad\nExec=reprise-notepad %F\n",
            &"notepad.desktop".parse().unwrap(),
        )
        .unwrap();
        let app = Target::App(Box::new(app));
        assert_eq!(app.default_name(), "Notepad");
        let text = app.launcher("Notes").unwrap();
        assert!(text.contains("Name=Notes\n") && text.contains("Icon=notepad\n"));
        assert!(text.contains("Exec=reprise-notepad\n"), "{text}");
    }

    #[test]
    fn a_web_address_offers_its_host_as_the_name() {
        let name = |url: &str| Target::Web(url.into()).default_name();
        assert_eq!(name("https://example.com"), "example.com");
        assert_eq!(
            name("https://www.example.com/a/b?q=1#top"),
            "www.example.com"
        );
        assert_eq!(name("http://u:pw@example.com:8080/"), "example.com");
        assert_eq!(name("http://[::1]:8080/x"), "[::1]");
        assert_eq!(
            name("ftp:///pub/x"),
            "ftp:---pub-x",
            "no host: the address, no /"
        );
        let root = Target::Path {
            path: "/".into(),
            is_dir: true,
        };
        assert_eq!(root.default_name(), "-");
    }

    #[test]
    fn a_shortcut_takes_a_free_name_and_is_trusted() {
        let tmp = tempfile::tempdir().unwrap();
        let text = Target::Command("htop".into()).launcher("htop").unwrap();
        let first = write(tmp.path(), "htop", &text).unwrap();
        let second = write(tmp.path(), "htop", &text).unwrap();
        assert_eq!(first, tmp.path().join("htop.desktop"));
        assert_eq!(second, tmp.path().join("htop (2).desktop"));
        assert!(apps::is_trusted(&second), "the desktop runs it");
    }
}
