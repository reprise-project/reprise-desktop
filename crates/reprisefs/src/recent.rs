//! Recently used files from the freedesktop `recently-used.xbel`, where
//! GTK and KDE record them.

use crate::apps::{self, AppEntry};
use std::ffi::OsString;
use std::path::PathBuf;

/// The recently used files; `current` parses the file again only when its mtime or size changed (it often runs to megabytes).
#[derive(Debug)]
pub struct RecentFiles {
    path: Option<PathBuf>,
    stamp: Option<(std::time::SystemTime, u64)>,
    parsed: Vec<Bookmark>,
}

impl Default for RecentFiles {
    fn default() -> RecentFiles {
        RecentFiles::new()
    }
}

/// Where the session's `recently-used.xbel` lives.
pub fn xbel_path() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    data.map(|d| d.join("recently-used.xbel"))
}

impl RecentFiles {
    pub fn new() -> RecentFiles {
        RecentFiles::at(xbel_path())
    }

    pub fn at(path: Option<PathBuf>) -> RecentFiles {
        RecentFiles {
            path,
            stamp: None,
            parsed: Vec::new(),
        }
    }

    pub fn current(&mut self, limit: usize) -> Vec<PathBuf> {
        self.used_by(limit, |_| true)
    }

    /// `current`, keeping only the files an application `entry` recorded
    /// using (see `used_by_app`): a taskbar button's jump list.
    pub fn for_app(&mut self, limit: usize, entry: &AppEntry) -> Vec<PathBuf> {
        self.used_by(limit, |b| used_by_entry(b, entry))
    }

    pub fn used_by(&mut self, limit: usize, keep: impl Fn(&Bookmark) -> bool) -> Vec<PathBuf> {
        self.bookmarks()
            .iter()
            .filter(|b| keep(b) && b.path.is_file())
            .take(limit)
            .map(|b| b.path.clone())
            .collect()
    }

    /// Every bookmark, newest first, whether or not its file still exists. Stats and reads only
    /// the xbel file.
    pub fn bookmarks(&mut self) -> &[Bookmark] {
        let Some(path) = &self.path else {
            return &[];
        };
        // Stat before reading: a write in between leaves an old stamp, so
        // the next call parses again.
        let stamp = std::fs::metadata(path)
            .ok()
            .and_then(|m| Some((m.modified().ok()?, m.len())));
        if stamp.is_none() || stamp != self.stamp {
            self.parsed = std::fs::read_to_string(path)
                .map(|t| parse_xbel(&t))
                .unwrap_or_default();
            self.stamp = stamp;
        }
        &self.parsed
    }
}

/// Whether the application `entry` recorded using bookmark `b` (see `used_by_app`).
pub fn used_by_entry(b: &Bookmark, entry: &AppEntry) -> bool {
    b.apps
        .iter()
        .any(|(name, exec)| used_by_app(name, exec, entry))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bookmark {
    pub path: PathBuf,
    pub apps: Vec<(String, String)>,
}

/// The `file://` bookmarks of an XBEL document, newest `modified` first,
/// each with its `<bookmark:application name=... exec=...>` entries. A
/// hand-rolled scan of the tags: the file is machine written and the
/// attributes we need are plain.
pub fn parse_xbel(text: &str) -> Vec<Bookmark> {
    let attr = |tag: &str, name: &str| -> Option<String> {
        let key = format!(" {name}=\"");
        let start = tag.find(&key)? + key.len();
        let end = tag[start..].find('"')? + start;
        Some(tag[start..end].to_string())
    };
    let mut found: Vec<(String, Bookmark)> = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("<bookmark ") {
        let tag_start = &rest[i..];
        let Some(end) = tag_start.find('>') else {
            break;
        };
        let tag = &tag_start[..end];
        // This bookmark's body runs to the next one.
        let body_end = tag_start[end..]
            .find("<bookmark ")
            .map_or(tag_start.len(), |j| end + j);
        let body = &tag_start[end..body_end];
        if let Some(href) = attr(tag, "href")
            && let Some(path) = href.strip_prefix("file://")
        {
            let modified = attr(tag, "modified").unwrap_or_default();
            let path: OsString =
                std::os::unix::ffi::OsStringExt::from_vec(crate::trash::percent_decode(path));
            let apps = body
                .split("<bookmark:application ")
                .skip(1)
                .filter_map(|app| {
                    let tag = &app[..app.find('>')?];
                    let tag = format!(" {tag}");
                    Some((
                        xml_unescape(&attr(&tag, "name").unwrap_or_default()),
                        xml_unescape(&attr(&tag, "exec").unwrap_or_default()),
                    ))
                })
                .collect();
            found.push((
                modified,
                Bookmark {
                    path: PathBuf::from(path),
                    apps,
                },
            ));
        }
        rest = &tag_start[body_end..];
    }
    // ISO 8601 timestamps sort as strings.
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().map(|(_, b)| b).collect()
}

fn xml_unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut rest = v;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        let Some(semi) = after.find(';') else {
            out.push_str(after);
            return out;
        };
        let name = &after[1..semi];
        let ch = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => name
                .strip_prefix("#x")
                .map(|h| u32::from_str_radix(h, 16).ok())
                .unwrap_or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => out.push(c),
            None => out.push_str(&after[..=semi]),
        }
        rest = &after[semi + 1..];
    }
    out.push_str(rest);
    out
}

/// Whether an xbel application entry (`name`, `exec` as recorded) is the
/// program `entry` starts: the program `exec` runs, or `name`, against the
/// program the entry's `Exec` runs or its `Name`, all case-insensitively.
/// See `program_of` for what counts as the program.
pub fn used_by_app(name: &str, exec: &str, entry: &AppEntry) -> bool {
    // GLib records exec shell-quoted as a whole: 'gedit %u'.
    let exec = exec.trim();
    let exec = exec
        .strip_prefix('\'')
        .and_then(|e| e.strip_suffix('\''))
        .unwrap_or(exec);
    let recorded = program_of(&apps::exec_argv(exec));
    let program = program_of(&apps::exec_argv(&entry.exec_line));
    let label = entry.name.to_lowercase();
    let name = name.trim().to_lowercase();
    let is_it = |w: &str| Some(w) == program.as_deref() || w == label;
    recorded.as_deref().is_some_and(is_it) || (!name.is_empty() && is_it(&name))
}

/// The program an Exec-style argument vector runs, lowercased: the first
/// word's file name, past a leading `env` with its options and VAR=value
/// assignments; for `flatpak run`, the app id after its options.
fn program_of(argv: &[String]) -> Option<String> {
    let base = |p: &str| p.rsplit('/').next().unwrap_or(p).to_lowercase();
    let mut args = argv.iter().map(String::as_str);
    let mut first = args.next()?;
    if base(first) == "env" {
        first = args.find(|a| !a.starts_with('-') && !a.contains('='))?;
    }
    if base(first) == "flatpak" {
        let mut rest = args.skip_while(|a| a.starts_with('-'));
        if rest.next() == Some("run") {
            return rest.find(|a| !a.starts_with('-')).map(str::to_lowercase);
        }
    }
    Some(base(first)).filter(|p| !p.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn recent_files_follow_the_xbel_file() {
        let dir = std::env::temp_dir().join(format!("reprise-recent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
        std::fs::write(&a, "").unwrap();
        std::fs::write(&b, "").unwrap();
        let xbel = dir.join("recently-used.xbel");
        let mark = |p: &Path, t: &str| {
            format!(
                r#"<bookmark href="file://{}" modified="{t}"/>"#,
                p.display()
            )
        };
        std::fs::write(&xbel, mark(&a, "1")).unwrap();
        let mut recent = RecentFiles::at(Some(xbel.clone()));
        assert_eq!(recent.current(10), vec![a.clone()]);
        std::fs::write(&xbel, mark(&a, "1") + &mark(&b, "2")).unwrap();
        assert_eq!(recent.current(10), [b.clone(), a.clone()]);
        assert_eq!(recent.current(1), vec![b.clone()]);
        std::fs::remove_file(&b).unwrap();
        assert_eq!(recent.current(10), vec![a.clone()]);
        std::fs::remove_file(&xbel).unwrap();
        assert!(recent.current(10).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recent_files_filter_by_the_application_that_used_them() {
        let dir = std::env::temp_dir().join(format!("reprise-recent-app-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = |n: &str| {
            let p = dir.join(n);
            std::fs::write(&p, "").unwrap();
            p
        };
        let (notes, todo, pic, both) = (
            file("notes.txt"),
            file("todo.txt"),
            file("pic.png"),
            file("both.txt"),
        );
        let mark = |p: &Path, t: &str, apps: &[(&str, &str)]| {
            let apps: String = apps
                .iter()
                .map(|(n, e)| {
                    format!(
                        "<bookmark:application name=\"{n}\" exec=\"{e}\" modified=\"{t}\" count=\"1\"/>"
                    )
                })
                .collect();
            format!(
                "<bookmark href=\"file://{}\" added=\"{t}\" modified=\"{t}\">\
                 <info><metadata owner=\"http://freedesktop.org\">\
                 <bookmark:applications>{apps}</bookmark:applications>\
                 </metadata></info></bookmark>\n",
                p.display()
            )
        };
        let xbel = dir.join("recently-used.xbel");
        let text = mark(
            &notes,
            "2026-01-01T00:00:00Z",
            &[("gedit", "&apos;gedit %u&apos;")],
        ) + &mark(
            &pic,
            "2026-01-02T00:00:00Z",
            &[("Image Viewer", "&apos;/usr/bin/eog %U&apos;")],
        ) + &mark(
            &todo,
            "2026-01-03T00:00:00Z",
            &[("org.gnome.gedit", "&apos;gedit %u&apos;")],
        ) + &mark(
            &both,
            "2026-01-04T00:00:00Z",
            &[("Image Viewer", "eog"), ("gedit", "gedit")],
        );
        std::fs::write(&xbel, text).unwrap();
        let entry = |name: &str, exec: &str| {
            apps::parse_desktop(
                &format!("[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\n"),
                "x.desktop",
            )
            .unwrap()
        };
        let gedit = entry("Text Editor", "gedit %U");
        let eog = entry("Image Viewer", "eog-wrapper %U");
        let mut recent = RecentFiles::at(Some(xbel));
        assert_eq!(
            recent.for_app(10, &gedit),
            [both.clone(), todo.clone(), notes.clone()]
        );
        assert_eq!(recent.for_app(1, &gedit), std::slice::from_ref(&both));
        // By Name: the recorded exec's program differs from the entry's.
        assert_eq!(recent.for_app(10, &eog), [both, pic]);
        assert!(recent.for_app(10, &entry("Terminal", "foot")).is_empty());
        // `env`, `flatpak run` and a quoted path with spaces name the
        // program they run.
        let wrapped = |exec: &str| used_by_app("", exec, &gedit);
        assert!(wrapped("'env LANG=C GTK_THEME=x gedit %u'"));
        assert!(wrapped("'/usr/bin/env -i PATH=/bin /usr/bin/gedit'"));
        assert!(wrapped("'\"/opt/My Apps/gedit\" %u'"));
        assert!(!wrapped("'env LANG=C eog %u'"));
        assert!(!wrapped("'\"/opt/My gedit/eog\" %u'"));
        let flat = entry(
            "Builder",
            "/usr/bin/flatpak run --branch=stable --arch=x86_64 --command=gnome-builder org.gnome.Builder @@u %U @@",
        );
        assert!(used_by_app("", "'flatpak run org.gnome.Builder %U'", &flat));
        assert!(used_by_app("org.gnome.Builder", "'gnome-builder'", &flat));
        assert!(!used_by_app("", "'flatpak run org.gnome.gedit'", &flat));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn xml_attribute_entities_decode() {
        assert_eq!(xml_unescape("&apos;a &amp; b&apos;"), "'a & b'");
        assert_eq!(xml_unescape("&#39;x&#x27; &lt;&gt;&quot;"), "'x' <>\"");
        assert_eq!(xml_unescape("a & b; &bogus;"), "a & b; &bogus;");
    }

    #[test]
    fn xbel_bookmarks_come_newest_first_and_decoded() {
        let text = r#"<?xml version="1.0"?>
<xbel version="1.0">
  <bookmark href="file:///home/u/old.txt" added="2026-01-01T00:00:00Z" modified="2026-01-01T00:00:00Z"/>
  <bookmark href="file:///home/u/a%20b.mp4" added="2026-03-01T00:00:00Z" modified="2026-03-05T10:00:00Z">
    <info/>
  </bookmark>
  <bookmark href="https://example.com/" modified="2026-04-01T00:00:00Z"/>
  <bookmark href="file:///home/u/mid.pdf" modified="2026-02-01T00:00:00Z"/>
</xbel>"#;
        let got: Vec<PathBuf> = parse_xbel(text).into_iter().map(|b| b.path).collect();
        assert_eq!(
            got,
            vec![
                PathBuf::from("/home/u/a b.mp4"),
                PathBuf::from("/home/u/mid.pdf"),
                PathBuf::from("/home/u/old.txt"),
            ]
        );
        assert!(parse_xbel("").is_empty());
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(
            parse_xbel(r#"<bookmark href="file:///home/u/a%FFb.txt" modified="x"/>"#)[0].path,
            Path::new(std::ffi::OsStr::from_bytes(b"/home/u/a\xffb.txt")),
            "a name that is not UTF-8 keeps its bytes"
        );
    }
}
