//! Recently used files in the freedesktop `recently-used.xbel`, where GTK and KDE record them
//! and [`record`] adds the files Reprise opens.

use crate::apps::{self, AppEntry};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

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
        if let Some(href) = attr(tag, "href").map(|h| xml_unescape(&h))
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

/// Bookmarks [`record`] keeps; the oldest by `modified` go first.
pub const MAX_BOOKMARKS: usize = 500;

/// A `recently-used.xbel` larger than this is left alone.
const MAX_XBEL_BYTES: u64 = 32 * 1024 * 1024;

const XBEL_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<xbel version=\"1.0\"\n      \
xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\"\n      \
xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\"\n>";

/// One writer at a time within the process; other programs replace the file whole, as this does.
static WRITING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A file that was opened, the desktop entry that opened it, and its mime type.
pub type Opening<'a> = (&'a Path, Option<&'a AppEntry>, Option<&'a str>);

/// Record each opened file that is a regular file in the session's `recently-used.xbel`, in one
/// rewrite of it.
pub fn record(opened: &[Opening]) -> std::io::Result<()> {
    let opened: Vec<_> = opened.iter().filter(|o| o.0.is_file()).copied().collect();
    if opened.is_empty() {
        return Ok(());
    }
    let xbel = xbel_path().ok_or_else(|| std::io::Error::other("no data directory"))?;
    record_in(&xbel, &opened, std::time::SystemTime::now())
}

/// [`record`] into the xbel file at `xbel`, stamped `now`. The file is read, edited and replaced
/// in one rename, so a reader sees the old file or the new one. GLib's writers take no lock
/// either, so a bookmark another program writes between the read and the rename is lost.
pub fn record_in(
    xbel: &Path,
    opened: &[Opening],
    now: std::time::SystemTime,
) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let _held = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let (old, mode) = match std::fs::File::open(xbel) {
        Ok(f) => {
            let bytes = crate::ops::read_opened(&f, MAX_XBEL_BYTES)?;
            let text = String::from_utf8(bytes)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            (text, f.metadata()?.permissions())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(dir) = xbel.parent() {
                std::fs::create_dir_all(dir)?;
            }
            (String::new(), std::fs::Permissions::from_mode(0o600))
        }
        Err(e) => return Err(e),
    };
    let stamp = iso8601(now);
    let mut new = old;
    for &(path, app, mime) in opened {
        let app = app.map(|e| (e.name.as_str(), format!("'{}'", e.exec_line)));
        let app = app.as_ref().map(|(n, e)| (*n, e.as_str()));
        new = recorded(&new, path, app, mime, &stamp).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "unreadable recently-used.xbel",
            )
        })?;
    }
    let (mut f, tmp) = crate::ops::create_temp_beside(xbel, Some(mode))?;
    let written = (|| {
        f.write_all(new.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, xbel)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// `t` in UTC as the xbel stamps write it: `2026-10-26T09:05:00.000000Z`.
fn iso8601(t: std::time::SystemTime) -> String {
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as libc::time_t);
    // SAFETY: `libc::tm` is plain data, and `gmtime_r` writes only the local `tm`.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::gmtime_r(&secs, &mut tm);
        tm
    };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000000Z",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

fn xml_escape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// The value of attribute `name` in the start tag `tag`, unescaped.
fn tag_attr(tag: &str, name: &str) -> Option<String> {
    let key = format!(" {name}=\"");
    let start = tag.find(&key)? + key.len();
    let end = tag[start..].find('"')? + start;
    Some(xml_unescape(&tag[start..end]))
}

/// The `<bookmark>` elements of `text` as byte ranges; None when one does not close.
fn bookmark_spans(text: &str) -> Option<Vec<std::ops::Range<usize>>> {
    let mut spans = Vec::new();
    let mut at = 0;
    while let Some(i) = text[at..].find("<bookmark ") {
        let start = at + i;
        let tag_end = start + text[start..].find('>')?;
        let end = if text[..tag_end].ends_with('/') {
            tag_end + 1
        } else {
            let close = "</bookmark>";
            tag_end + text[tag_end..].find(close)? + close.len()
        };
        spans.push(start..end);
        at = end;
    }
    Some(spans)
}

const FREEDESKTOP_METADATA: &str = "<metadata owner=\"http://freedesktop.org\">";

fn mime_entry(mime: &str) -> String {
    format!(
        "\n          <mime:mime-type type=\"{}\"/>",
        xml_escape(mime)
    )
}

fn app_entry(name: &str, exec: &str, stamp: &str, count: u64) -> String {
    format!(
        "<bookmark:application name=\"{}\" exec=\"{}\" modified=\"{stamp}\" count=\"{count}\"/>",
        xml_escape(name),
        xml_escape(exec)
    )
}

/// Sets attribute `name` in `tag` (a start tag without its `>`) to `value`, an escaped string.
fn set_attr(tag: &mut String, name: &str, value: &str) {
    let key = format!(" {name}=\"");
    if let Some(i) = tag.find(&key) {
        let start = i + key.len();
        if let Some(len) = tag[start..].find('"') {
            tag.replace_range(start..start + len, value);
            return;
        }
    }
    tag.push_str(&format!(" {name}=\"{value}\""));
}

/// The `<bookmark:application .../>` entry for `name` in `body`; None when an entry does not
/// close.
fn app_span(body: &str, name: &str) -> Option<Option<std::ops::Range<usize>>> {
    let mut at = 0;
    while let Some(i) = body[at..].find("<bookmark:application ") {
        let start = at + i;
        let end = start + body[start..].find("/>")? + 2;
        if tag_attr(&body[start..end], "name").as_deref() == Some(name) {
            return Some(Some(start..end));
        }
        at = end;
    }
    Some(None)
}

/// `el` with the application entries of `others`, more elements for its href, merged in: a
/// program `el` lists adds the other count to its own, another is appended. None when `el` has
/// no applications list or an entry does not close.
fn merged(el: &str, others: &[&str]) -> Option<String> {
    let count = |e: &str| tag_attr(e, "count").and_then(|c| c.parse::<u64>().ok());
    let mut el = el.to_string();
    for other in others {
        for part in other.split("<bookmark:application ").skip(1) {
            let entry = format!("<bookmark:application {}/>", &part[..part.find("/>")?]);
            let Some(name) = tag_attr(&entry, "name") else {
                continue;
            };
            if let Some(r) = app_span(&el, &name)? {
                let mut tag = el[r.start..r.end - 2].to_string();
                let sum = count(&el[r.clone()]).unwrap_or(0) + count(&entry).unwrap_or(0);
                set_attr(&mut tag, "count", &sum.to_string());
                el.replace_range(r, &format!("{tag}/>"));
            } else {
                let i = el.find("</bookmark:applications>")?;
                el.insert_str(i, &format!("  {entry}\n          "));
            }
        }
    }
    Some(el)
}

/// The bookmark element `el` edited in place for one more use by `app` at `stamp`: new
/// `modified`/`visited`, `app`'s entry added or its count bumped, `mime` added when it has none.
/// None for a self-closing element or one the edit cannot follow.
fn spliced(el: &str, app: Option<(&str, &str)>, mime: Option<&str>, stamp: &str) -> Option<String> {
    let tag_end = el.find('>')?;
    if el[..tag_end].ends_with('/') {
        return None;
    }
    let mut tag = el[..tag_end].to_string();
    set_attr(&mut tag, "modified", stamp);
    set_attr(&mut tag, "visited", stamp);
    let mut body = el[tag_end..].to_string();
    let mut meta = String::new();
    if let Some(m) = mime.filter(|_| !body.contains("<mime:mime-type")) {
        meta.push_str(&mime_entry(m));
    }
    if let Some((name, exec)) = app {
        if let Some(r) = app_span(&body, name)? {
            let count: u64 = tag_attr(&body[r.clone()], "count")
                .and_then(|c| c.parse().ok())
                .unwrap_or(0);
            body.replace_range(r, &app_entry(name, exec, stamp, count + 1));
        } else if let Some(i) = body.find("</bookmark:applications>") {
            body.insert_str(
                i,
                &format!("  {}\n          ", app_entry(name, exec, stamp, 1)),
            );
        } else {
            meta.push_str(&format!(
                "\n          <bookmark:applications>\n            {}\n          </bookmark:applications>",
                app_entry(name, exec, stamp, 1)
            ));
        }
    }
    if !meta.is_empty() {
        if let Some(i) = body.find(FREEDESKTOP_METADATA) {
            body.insert_str(i + FREEDESKTOP_METADATA.len(), &meta);
        } else if let Some(i) = body.find("</info>") {
            body.insert_str(
                i,
                &format!("  {FREEDESKTOP_METADATA}{meta}\n      </metadata>\n    "),
            );
        } else {
            let i = body.rfind("</bookmark>")?;
            body.insert_str(
                i,
                &format!("  <info>\n      {FREEDESKTOP_METADATA}{meta}\n      </metadata>\n    </info>\n  "),
            );
        }
    }
    Some(tag + &body)
}

/// `old` (an xbel document, or empty) with `path` recorded at `stamp` as opened by `app`
/// (`(name, exec)`): a bookmark already there for it keeps everything it holds (title, groups,
/// `private`, other programs) and gets new `modified`/`visited` times and `app`'s count up by one. Past [`MAX_BOOKMARKS`] the
/// oldest bookmarks go. None when `old` cannot be read as xbel.
pub fn recorded(
    old: &str,
    path: &Path,
    app: Option<(&str, &str)>,
    mime: Option<&str>,
    stamp: &str,
) -> Option<String> {
    let uri = crate::thumbnails::file_uri(path);
    let spans = if old.trim().is_empty() {
        Vec::new()
    } else {
        old.find("<xbel")?;
        bookmark_spans(old)?
    };
    let head = match spans.first() {
        Some(s) => old[..s.start].trim_end(),
        None => old.find("</xbel>").map_or("", |i| old[..i].trim_end()),
    };
    let head = if head.contains("xmlns:bookmark=") && head.contains("xmlns:mime=") {
        head
    } else {
        XBEL_HEAD
    };
    let mut added = stamp.to_string();
    let mut apps: Vec<String> = Vec::new();
    let mut count = 0u64;
    let mut kept: Vec<(String, &str)> = Vec::new();
    let mut matched: Option<&str> = None;
    let mut others: Vec<&str> = Vec::new();
    for span in &spans {
        let el = &old[span.clone()];
        let tag = &el[..el.find('>').unwrap_or(el.len())];
        if tag_attr(tag, "href").as_deref() != Some(&uri) {
            kept.push((tag_attr(tag, "modified").unwrap_or_default(), el));
            continue;
        }
        if matched.is_some() {
            others.push(el);
        }
        matched.get_or_insert(el);
        added = tag_attr(tag, "added").unwrap_or(added);
        for part in el.split("<bookmark:application ").skip(1) {
            let Some(end) = part.find("/>") else { continue };
            let entry = format!(" {}", &part[..end]);
            if app.is_some_and(|(name, _)| tag_attr(&entry, "name").as_deref() == Some(name)) {
                count = tag_attr(&entry, "count")
                    .and_then(|c| c.parse().ok())
                    .unwrap_or(0);
            } else {
                apps.push(format!("<bookmark:application{}/>", entry.trim_end()));
            }
        }
    }
    if let Some((name, exec)) = app {
        apps.push(app_entry(name, exec, stamp, count + 1));
    }
    let mime_line = mime.map_or_else(String::new, mime_entry);
    let apps: String = apps.iter().map(|a| format!("\n            {a}")).collect();
    let apps = if apps.is_empty() {
        String::new()
    } else {
        format!("\n          <bookmark:applications>{apps}\n          </bookmark:applications>")
    };
    let ours = matched
        .and_then(|el| merged(el, &others))
        .and_then(|el| spliced(&el, app, mime, stamp))
        .unwrap_or_else(|| {
            format!(
                "<bookmark href=\"{}\" added=\"{added}\" modified=\"{stamp}\" visited=\"{stamp}\">\n    \
                 <info>\n      <metadata owner=\"http://freedesktop.org\">{mime_line}{apps}\n      \
                 </metadata>\n    </info>\n  </bookmark>",
                xml_escape(&uri)
            )
        });
    if kept.len() >= MAX_BOOKMARKS {
        let mut newest: Vec<usize> = (0..kept.len()).collect();
        newest.sort_by(|&a, &b| kept[b].0.cmp(&kept[a].0));
        let mut keep = vec![false; kept.len()];
        for &i in &newest[..MAX_BOOKMARKS - 1] {
            keep[i] = true;
        }
        let mut i = 0;
        kept.retain(|_| {
            i += 1;
            keep[i - 1]
        });
    }
    let mut out = String::from(head);
    for (_, el) in &kept {
        out.push_str("\n  ");
        out.push_str(el);
    }
    out.push_str("\n  ");
    out.push_str(&ours);
    out.push_str("\n</xbel>\n");
    Some(out)
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

    fn editor() -> AppEntry {
        apps::parse_desktop(
            "[Desktop Entry]\nType=Application\nName=Text & Notes\nExec=notes %U\n",
            "notes.desktop",
        )
        .unwrap()
    }

    #[test]
    fn a_recorded_file_parses_back_with_its_program() {
        let dir = std::env::temp_dir().join(format!("reprise-recent-rec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let doc = dir.join("Fish & chips.txt");
        std::fs::write(&doc, "").unwrap();
        let xbel = dir.join("share/recently-used.xbel");
        let at = |s: u64| std::time::UNIX_EPOCH + std::time::Duration::from_secs(s);
        let notes = editor();
        record_in(
            &xbel,
            &[(&doc, Some(&notes), Some("text/plain"))],
            at(1_800_000_000),
        )
        .unwrap();
        let text = std::fs::read_to_string(&xbel).unwrap();
        assert!(
            text.contains("href=\"file://") && text.contains("Fish%20&amp;%20chips.txt\""),
            "the space is percent-encoded and the & escaped: {text}"
        );
        assert!(text.contains("name=\"Text &amp; Notes\""), "{text}");
        assert!(
            text.contains("<mime:mime-type type=\"text/plain\"/>"),
            "{text}"
        );
        assert!(
            text.contains("modified=\"2027-01-15T08:00:00.000000Z\""),
            "{text}"
        );
        let mut recent = RecentFiles::at(Some(xbel.clone()));
        assert_eq!(recent.current(10), vec![doc.clone()]);
        assert_eq!(recent.for_app(10, &notes), vec![doc.clone()]);

        let other = dir.join("other.txt");
        std::fs::write(&other, "").unwrap();
        record_in(&xbel, &[(&other, None, None)], at(1_800_000_100)).unwrap();
        record_in(&xbel, &[(&doc, Some(&notes), None)], at(1_800_000_200)).unwrap();
        let text = std::fs::read_to_string(&xbel).unwrap();
        assert_eq!(
            text.matches("<bookmark ").count(),
            2,
            "one bookmark per file: {text}"
        );
        assert!(
            text.contains("count=\"2\""),
            "a second open counts up: {text}"
        );
        assert!(
            text.contains("added=\"2027-01-15T08:00:00.000000Z\""),
            "{text}"
        );
        assert_eq!(recent.current(10), [doc.clone(), other.clone()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_record_takes_every_opened_file() {
        let dir = std::env::temp_dir().join(format!("reprise-recent-all-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
        std::fs::write(&a, "").unwrap();
        std::fs::write(&b, "").unwrap();
        let xbel = dir.join("recently-used.xbel");
        let notes = editor();
        let now = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000);
        record_in(
            &xbel,
            &[(&a, Some(&notes), None), (&b, Some(&notes), None)],
            now,
        )
        .unwrap();
        let mut recent = RecentFiles::at(Some(xbel));
        let mut got = recent.for_app(10, &notes);
        got.sort();
        assert_eq!(got, [a, b]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recording_keeps_other_programs_and_caps_the_bookmarks() {
        let gtk = r#"<?xml version="1.0" encoding="UTF-8"?>
<xbel version="1.0"
      xmlns:bookmark="http://www.freedesktop.org/standards/desktop-bookmarks"
      xmlns:mime="http://www.freedesktop.org/standards/shared-mime-info"
>
  <bookmark href="file:///home/u/a.txt" added="2026-01-01T00:00:00Z" modified="2026-01-01T00:00:00Z" visited="2026-01-01T00:00:00Z">
    <info>
      <metadata owner="http://freedesktop.org">
        <mime:mime-type type="text/plain"/>
        <bookmark:applications>
          <bookmark:application name="gedit" exec="&apos;gedit %u&apos;" modified="2026-01-01T00:00:00Z" count="3"/>
        </bookmark:applications>
      </metadata>
    </info>
  </bookmark>
</xbel>"#;
        let notes = editor();
        let exec = format!("'{}'", notes.exec_line);
        let app = Some((notes.name.as_str(), exec.as_str()));
        let a = Path::new("/home/u/a.txt");
        let out = recorded(gtk, a, app, Some("text/plain"), "2026-02-01T00:00:00Z").unwrap();
        let marks = parse_xbel(&out);
        assert_eq!(marks.len(), 1);
        let names: Vec<&str> = marks[0].apps.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["gedit", "Text & Notes"]);
        assert!(out.contains("count=\"3\""), "gedit's count stays: {out}");
        assert!(out.contains("added=\"2026-01-01T00:00:00Z\""), "{out}");

        let mut many = String::new();
        for i in 0..MAX_BOOKMARKS + 5 {
            let stamp = format!("2026-01-01T00:{:02}:{:02}Z", i / 60, i % 60);
            let mark = format!("<bookmark href=\"file:///f/{i}\" modified=\"{stamp}\"/>\n");
            many.push_str(&mark);
        }
        let doc = format!("{}\n{many}</xbel>", XBEL_HEAD);
        let out = recorded(
            &doc,
            Path::new("/f/new"),
            None,
            None,
            "2027-01-01T00:00:00Z",
        )
        .unwrap();
        let marks = parse_xbel(&out);
        assert_eq!(marks.len(), MAX_BOOKMARKS);
        assert_eq!(marks[0].path, Path::new("/f/new"));
        assert!(
            marks.iter().all(|b| b.path != Path::new("/f/5")),
            "the oldest go"
        );
        assert!(marks.iter().any(|b| b.path == Path::new("/f/6")));
        assert!(recorded("<xbel><bookmark href=\"x\">", a, None, None, "t").is_none());
    }

    #[test]
    fn an_updated_bookmark_keeps_its_title_groups_and_private_flag() {
        let gimp = format!(
            r#"{XBEL_HEAD}
  <bookmark href="file:///home/u/p.png" added="2026-01-01T00:00:00Z" modified="2026-01-01T00:00:00Z" visited="2026-01-01T00:00:00Z">
    <title>Holiday</title>
    <info>
      <metadata owner="http://freedesktop.org">
        <mime:mime-type type="image/png"/>
        <bookmark:groups>
          <bookmark:group>Graphics</bookmark:group>
        </bookmark:groups>
        <bookmark:applications>
          <bookmark:application name="GNU Image Manipulation Program" exec="&apos;gimp %U&apos;" modified="2026-01-01T00:00:00Z" count="2"/>
          <bookmark:application name="Text &amp; Notes" exec="&apos;notes&apos;" modified="2026-01-01T00:00:00Z" count="4"/>
        </bookmark:applications>
        <bookmark:private/>
      </metadata>
    </info>
  </bookmark>
</xbel>"#
        );
        let notes = editor();
        let exec = format!("'{}'", notes.exec_line);
        let app = Some((notes.name.as_str(), exec.as_str()));
        let p = Path::new("/home/u/p.png");
        let out = recorded(&gimp, p, app, Some("text/plain"), "2026-02-01T00:00:00Z").unwrap();
        for kept in [
            "<title>Holiday</title>",
            "<bookmark:group>Graphics</bookmark:group>",
            "<bookmark:private/>",
            "count=\"2\"",
            "added=\"2026-01-01T00:00:00Z\"",
            "<mime:mime-type type=\"image/png\"/>",
        ] {
            assert!(out.contains(kept), "{kept} is kept: {out}");
        }
        assert!(
            !out.contains("text/plain"),
            "an existing mime type stays: {out}"
        );
        assert!(
            out.contains("count=\"5\""),
            "our count goes from 4 to 5: {out}"
        );
        assert!(out.contains("visited=\"2026-02-01T00:00:00Z\""), "{out}");
        let marks = parse_xbel(&out);
        assert_eq!(marks.len(), 1);
        let names: Vec<&str> = marks[0].apps.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["GNU Image Manipulation Program", "Text & Notes"]);

        let other = Some(("Viewer", "'viewer %f'"));
        let out = recorded(&gimp, p, other, None, "2026-02-01T00:00:00Z").unwrap();
        let names: Vec<String> = parse_xbel(&out)[0]
            .apps
            .iter()
            .map(|(n, _)| n.clone())
            .collect();
        assert_eq!(names.len(), 3, "a new program joins the list: {out}");
        assert!(out.contains("<bookmark:private/>"));
    }

    #[test]
    fn two_bookmarks_for_one_file_become_one_holding_both_programs() {
        let mark = |apps: &str| {
            format!(
                r#"<bookmark href="file:///home/u/p.png" added="1" modified="1" visited="1">
    <info>
      <metadata owner="http://freedesktop.org">
        <bookmark:applications>
          {apps}
        </bookmark:applications>
      </metadata>
    </info>
  </bookmark>"#
            )
        };
        let entry = |name: &str, count: u32| {
            format!(
                r#"<bookmark:application name="{name}" exec="'{name}'" modified="1" count="{count}"/>"#
            )
        };
        let old = format!(
            "{XBEL_HEAD}\n  {}\n  {}\n</xbel>",
            mark(&(entry("Gimp", 2) + &entry("Viewer", 1))),
            mark(&(entry("Viewer", 3) + &entry("Paint", 5)))
        );
        let p = Path::new("/home/u/p.png");
        let out = recorded(&old, p, Some(("Gimp", "'Gimp'")), None, "2").unwrap();
        let marks = parse_xbel(&out);
        assert_eq!(marks.len(), 1, "{out}");
        let names: Vec<&str> = marks[0].apps.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["Gimp", "Viewer", "Paint"]);
        for count in [r#"count="3""#, r#"count="4""#, r#"count="5""#] {
            assert!(out.contains(count), "{count}: {out}");
        }
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
