//! New > Shortcut's targets. A program from the list, an existing file or folder, or a command
//! line becomes a `Type=Application` launcher (a file opening through `xdg-open`, a folder in
//! Explorer, as Create Shortcut's launchers do); a web address becomes a `Type=Link`. The file is
//! written trusted, as the desktop runs only trusted launchers.

use reprisekit::controls::newshortcut::NAME_MAX_BYTES;
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

    /// The launcher's text, shown as `name`, a web address with icon `browser` (else
    /// `text-html`); None for a path or command a desktop entry cannot hold (a control
    /// character, a path that is not UTF-8).
    pub fn launcher(&self, name: &str, browser: Option<&str>) -> Option<String> {
        let (icon, command) = match self {
            Target::Web(url) => {
                return apps::link_text_with(name, url, browser.unwrap_or("text-html"));
            }
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

/// The icon of the default web browser, for a web shortcut: the program the mime-apps lists open
/// https (else http) links in.
pub fn browser_icon(apps: &[AppEntry]) -> Option<String> {
    ["x-scheme-handler/https", "x-scheme-handler/http"]
        .into_iter()
        .find_map(|mime| crate::props::default_app_entry(mime, apps)?.icon.clone())
        .filter(|icon| !icon.contains(char::is_control))
}

/// Whether `url` is an http or https address, the ones a browser opens and a title is read from.
pub fn is_http(url: &str) -> bool {
    url.split_once("://").is_some_and(|(scheme, _)| {
        ["http", "https"].contains(&scheme.to_ascii_lowercase().as_str())
    })
}

/// How long the title fetch may take in all, connecting and redirects included, in seconds.
const TITLE_WAIT: &str = "3";
/// How much of the page the title is looked for in.
const TITLE_READ: u64 = 8 * 1024;
/// The longest name a title gives, in characters; [`NAME_MAX_BYTES`] caps it too.
const TITLE_MAX: usize = 100;

/// The `<title>` of the http or https page at `url`, made a shortcut name: read by curl within
/// [`TITLE_WAIT`] from the page's first [`TITLE_READ`] bytes, which are only searched as text.
/// None offline, on an error status, past the wait, or without a title. Blocks: call it off the
/// UI thread.
pub fn page_title(url: &str) -> Option<String> {
    if !is_http(url) {
        return None;
    }
    let mut child = std::process::Command::new("curl")
        .args(curl_args(url))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut head = Vec::new();
    let read = child.stdout.take().map(|out| {
        use std::io::Read;
        out.take(TITLE_READ).read_to_end(&mut head)
    });
    let _ = child.kill();
    let _ = child.wait();
    read?.ok()?;
    title_in(&head)
}

/// curl's arguments for [`page_title`]: no `.curlrc` (`-q` first), `url` taken as written rather
/// than as a glob, which would turn `[`, `]`, `{` and `}` into many requests.
fn curl_args(url: &str) -> Vec<&str> {
    vec![
        "-q",
        "--globoff",
        "--silent",
        "--fail",
        "--location",
        "--max-redirs",
        "5",
        "--proto",
        "=http,https",
        "--proto-redir",
        "=http,https",
        "--max-time",
        TITLE_WAIT,
        url,
    ]
}

/// The text of the first `<title>` element in `html`, entities decoded, hidden characters
/// dropped, blanks collapsed, leading dots (a hidden file) removed, a `/` made `-`, and cut to
/// [`TITLE_MAX`] characters and [`NAME_MAX_BYTES`]; None when there is none or it is blank.
fn title_in(html: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(html);
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    let open = loop {
        let at = from + lower[from..].find("<title")?;
        let next = lower[at + 6..].chars().next()?;
        if next == '>' || next.is_ascii_whitespace() {
            break at;
        }
        from = at + 6;
    };
    let start = open + lower[open..].find('>')? + 1;
    let end = start + lower[start..].find("</title")?;
    let words = decode_entities(&text[start..end]);
    let words: String = words.chars().filter(|&c| !hidden(c)).collect();
    let joined = words.split_whitespace().collect::<Vec<_>>().join(" ");
    let joined = joined.trim_start_matches(['.', ' ']).replace('/', "-");
    let mut name = String::new();
    for c in joined.chars().take(TITLE_MAX) {
        if name.len() + c.len_utf8() > NAME_MAX_BYTES {
            break;
        }
        name.push(c);
    }
    let name = name.trim_end().to_string();
    (!name.is_empty()).then_some(name)
}

/// A character a name must not carry: a control, or a bidi or zero-width format character, which
/// would turn or pad the label unseen. The zero-width joiner and non-joiner stay: scripts and
/// emoji sequences spell with them.
fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061C}' | '\u{200B}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}'
        )
        || matches!(c, '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

/// `s` with HTML's character references decoded: numeric ones and the named ones titles use;
/// any other `&` stays as written.
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let decoded = rest[1..].find(';').filter(|&n| n <= 10).and_then(|n| {
            let name = &rest[1..1 + n];
            let c = match name {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                "nbsp" => ' ',
                "ndash" => '\u{2013}',
                "mdash" => '\u{2014}',
                "laquo" => '\u{AB}',
                "raquo" => '\u{BB}',
                "lsquo" => '\u{2018}',
                "rsquo" => '\u{2019}',
                "ldquo" => '\u{201C}',
                "rdquo" => '\u{201D}',
                "hellip" => '\u{2026}',
                "middot" => '\u{B7}',
                "bull" => '\u{2022}',
                "copy" => '\u{A9}',
                "reg" => '\u{AE}',
                "trade" => '\u{2122}',
                _ => {
                    let num = name.strip_prefix('#')?;
                    let code = match num.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => num.parse().ok()?,
                    };
                    char::from_u32(code)?
                }
            };
            Some((c, n + 2))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
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
        let text = folder.launcher("Music", None).unwrap();
        assert!(text.starts_with("[Desktop Entry]\nType=Application\nName=Music\n"));
        assert!(
            text.contains("Icon=folder\n")
                && text.contains("Exec=reprise-explorer /home/u/Music\n")
        );
        let file = Target::Path {
            path: "/home/u/a b.txt".into(),
            is_dir: false,
        };
        let text = file.launcher("a", None).unwrap();
        assert!(text.contains("Exec=xdg-open \"/home/u/a b.txt\"\n"));
        assert!(
            text.contains("Icon=text-x-generic\n"),
            "a name every theme carries"
        );
        let cmd = Target::Command("date +%s".into());
        assert_eq!(cmd.default_name(), "date");
        assert!(
            cmd.launcher("Date", None)
                .unwrap()
                .contains("Exec=date +%%s\n"),
            "a % is no field code"
        );
        let web = Target::Web("https://example.org".into());
        assert!(
            web.launcher("Example", None)
                .unwrap()
                .contains("Type=Link\n")
        );
        let app = apps::parse_desktop(
            "[Desktop Entry]\nType=Application\nName=Notepad\nIcon=notepad\nExec=reprise-notepad %F\n",
            &"notepad.desktop".parse().unwrap(),
        )
        .unwrap();
        let app = Target::App(Box::new(app));
        assert_eq!(app.default_name(), "Notepad");
        let text = app.launcher("Notes", None).unwrap();
        assert!(text.contains("Name=Notes\n") && text.contains("Icon=notepad\n"));
        assert!(text.contains("Exec=reprise-notepad\n"), "{text}");
    }

    #[test]
    fn a_page_title_becomes_a_name_and_only_the_title_is_read() {
        let page = |head: &str| title_in(head.as_bytes());
        assert_eq!(
            page("<html><head><TITLE>\n  Docs  &amp; Gui").as_deref(),
            None,
            "cut before its end"
        );
        assert_eq!(
            page("<head><meta charset=utf-8><Title lang=en>Docs &amp;\n Guides &#8212; Ex&#x41;mple</title>")
                .as_deref(),
            Some("Docs & Guides \u{2014} ExAmple")
        );
        assert_eq!(
            page("<titles>no</titles><title>A/B &unknown; &#xZZ;</title>").as_deref(),
            Some("A-B &unknown; &#xZZ;"),
            "a slash would grey Finish"
        );
        assert_eq!(page("<script>document.title='x'</script>"), None);
        assert_eq!(page("<title> \n </title>"), None, "blank");
        let long = format!("<title>{}</title>", "é".repeat(300));
        assert_eq!(page(&long).map(|t| t.chars().count()), Some(TITLE_MAX));
    }

    #[test]
    fn a_title_loses_hidden_characters_and_leading_dots_and_fits_a_file_name() {
        let page = |head: &str| title_in(head.as_bytes());
        assert_eq!(
            page("<title>Blog &raquo; Post &ndash; A&hellip; &copy;</title>").as_deref(),
            Some("Blog \u{BB} Post \u{2013} A\u{2026} \u{A9}")
        );
        assert_eq!(
            page("<title>\u{202E}evil&#x202E;\u{200B} \u{FEFF}name</title>").as_deref(),
            Some("evil name")
        );
        assert_eq!(page("<title>a \u{1} b</title>").as_deref(), Some("a b"));
        assert_eq!(page("<title>. News</title>").as_deref(), Some("News"));
        assert_eq!(page("<title>...</title>"), None, "only dots");
        let cjk = page(&format!("<title>{}</title>", "新聞".repeat(50))).expect("a name");
        assert!(cjk.len() <= NAME_MAX_BYTES, "{} bytes", cjk.len());
        assert_eq!(cjk.chars().count(), NAME_MAX_BYTES / 3, "whole characters");
    }

    #[test]
    fn curl_takes_the_address_as_written_not_as_a_glob() {
        let url = "https://shop.example/items?filter[status]=open&ids[]=1{a,b}";
        let args = curl_args(url);
        assert_eq!(&args[..2], ["-q", "--globoff"], "no .curlrc, no globbing");
        assert_eq!(args.last(), Some(&url));
    }

    #[test]
    fn only_a_web_page_is_fetched_for_its_title() {
        assert_eq!(page_title("ftp://example.org/"), None);
        assert_eq!(page_title("file:///etc/hostname"), None);
        assert_eq!(page_title("example.org"), None);
    }

    #[test]
    fn only_http_and_https_count_as_a_browsers_address() {
        assert!(is_http("HTTPS://example.org/"));
        assert!(is_http("http://example.org/"));
        assert!(!is_http("ftp://example.org/"));
        assert!(!is_http("example.org"));
    }

    #[test]
    fn a_web_shortcut_shows_the_browsers_icon() {
        let web = Target::Web("https://example.org/".into());
        let text = web.launcher("Example", Some("firefox")).unwrap();
        assert!(text.contains("\nIcon=firefox\n"), "{text}");
        let text = web.launcher("Example", None).unwrap();
        assert!(text.contains("\nIcon=text-html\n"), "{text}");
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
        let text = Target::Command("htop".into())
            .launcher("htop", None)
            .unwrap();
        let first = write(tmp.path(), "htop", &text).unwrap();
        let second = write(tmp.path(), "htop", &text).unwrap();
        assert_eq!(first, tmp.path().join("htop.desktop"));
        assert_eq!(second, tmp.path().join("htop (2).desktop"));
        assert!(apps::is_trusted(&second), "the desktop runs it");
    }
}
