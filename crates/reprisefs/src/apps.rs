//! Application entries from freedesktop .desktop files: the Start menu,
//! Open With, and a window's icon by its app_id (the share picker).

use reprisebus::mutex::lock;
use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppEntry {
    pub name: String,
    /// `Exec=` as written, field codes and all: [`launch_argv`] and
    /// [`open_argv`] split it with no shell.
    pub exec_line: String,
    pub terminal: bool,
    pub id: String,
    /// A theme icon name or an absolute path.
    pub icon: Option<String>,
    pub startup_wm_class: Option<String>,
    pub categories: Vec<String>,
    pub mime_types: Vec<String>,
    /// False for `NoDisplay=true`: opens files and names a window's icon but
    /// stays out of the menus, launcher and Open With lists (unless it is
    /// the type's default or [`AppEntry::is_browsed`]).
    pub shown: bool,
}

impl AppEntry {
    pub fn in_category(&self, category: &str) -> bool {
        self.categories.iter().any(|c| c == category)
    }

    pub fn is_browsed(&self) -> bool {
        self.id.starts_with(BROWSED_PREFIX)
    }
}

const BROWSED_PREFIX: &str = "reprise-userapp-";

const DESKTOP_NAME: &str = "reprise";

/// Remove %-field codes ("%%" becomes "%"). Whitespace between arguments
/// collapses to one space; inside double quotes it stays.
pub fn strip_field_codes(exec: &str) -> String {
    let mut out = String::with_capacity(exec.len());
    let mut chars = exec.chars().peekable();
    let mut quoted = false;
    let mut pending_space = false;
    while let Some(c) = chars.next() {
        match c {
            '%' => {
                if let Some('%') = chars.next() {
                    if pending_space && !out.is_empty() {
                        out.push(' ');
                    }
                    pending_space = false;
                    out.push('%');
                }
            }
            '"' => {
                if pending_space && !out.is_empty() {
                    out.push(' ');
                }
                pending_space = false;
                quoted = !quoted;
                out.push(c);
            }
            '\\' if quoted => {
                out.push(c);
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            c if c.is_whitespace() && !quoted => pending_space = true,
            c => {
                if pending_space && !out.is_empty() {
                    out.push(' ');
                }
                pending_space = false;
                out.push(c);
            }
        }
    }
    out
}

/// `exec` as the argument vector the desktop-entry spec describes: split
/// on unquoted whitespace, double quotes group an argument, and inside
/// them a backslash escapes `"`, `` ` ``, `$` and `\`. Field codes are
/// dropped and `%%` is a percent. Not a shell line: `$HOME`, backticks
/// and `;` are plain characters.
pub fn exec_argv(exec: &str) -> Vec<String> {
    // Every byte comes from `exec`, so the lossy step never replaces one.
    split_exec(exec, |_| Vec::new())
        .into_iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

/// `exec`'s first argument as [`exec_argv`] reads it, and the unparsed rest
/// of the line after it.
pub fn exec_program_and_rest(exec: &str) -> (String, &str) {
    let exec = exec.trim_start();
    let mut quoted = false;
    let mut end = exec.len();
    let mut chars = exec.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => quoted = !quoted,
            '\\' if quoted => {
                chars.next();
            }
            c if c.is_whitespace() && !quoted => {
                end = i;
                break;
            }
            _ => {}
        }
    }
    let program = exec_argv(&exec[..end]).pop().unwrap_or_default();
    (program, exec[end..].trim())
}

/// `arg` written so [`exec_argv`] reads it back as one argument.
pub fn exec_quote(arg: &str) -> String {
    let arg = arg.replace('%', "%%");
    if !arg.is_empty() && !arg.contains(|c: char| c.is_whitespace() || "\"`$\\".contains(c)) {
        return arg;
    }
    let mut out = String::from("\"");
    for c in arg.chars() {
        if "\"`$\\".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// [`exec_argv`]'s splitting, with field code `%c` replaced by `field('c')`: one value goes
/// inside its argument, several stand as arguments of their own (`%i`), and none drops it, an
/// argument that was only the code going with it.
fn split_exec(exec: &str, mut field: impl FnMut(char) -> Vec<OsString>) -> Vec<OsString> {
    let mut argv = Vec::new();
    let mut arg = OsString::new();
    let mut have = false;
    let mut quoted = false;
    let mut buf = [0u8; 4];
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                have = true;
            }
            '\\' if quoted => match chars.next() {
                Some(n @ ('"' | '`' | '$' | '\\')) => arg.push(n.encode_utf8(&mut buf)),
                Some(n) => {
                    arg.push("\\");
                    arg.push(n.encode_utf8(&mut buf));
                }
                None => arg.push("\\"),
            },
            '%' => match chars.next() {
                Some('%') => {
                    arg.push("%");
                    have = true;
                }
                Some(code) => {
                    let mut values = field(code);
                    if values.len() == 1 {
                        arg.push(values.remove(0));
                        have = true;
                    } else if !values.is_empty() {
                        if have {
                            argv.push(std::mem::take(&mut arg));
                            have = false;
                        }
                        argv.extend(values);
                    }
                }
                None => {}
            },
            c if c.is_whitespace() && !quoted => {
                if have {
                    argv.push(std::mem::take(&mut arg));
                    have = false;
                }
            }
            c => {
                arg.push(c.encode_utf8(&mut buf));
                have = true;
            }
        }
    }
    if have {
        argv.push(arg);
    }
    argv
}

fn list_contains(value: &str, needle: &str) -> bool {
    value.split(';').any(|s| s.trim() == needle)
}

const DESKTOP_FILE_MAX: u64 = 64 * 1024;

/// The text of the `.desktop` file at `path`, following symlinks. Only a
/// regular file of at most 64 KiB is read, and the open does not block, so
/// a FIFO or device named `*.desktop` fails instead of hanging the caller.
pub fn read_desktop_file(path: &Path) -> std::io::Result<String> {
    let bytes = crate::ops::read_regular(path, DESKTOP_FILE_MAX)?;
    String::from_utf8(bytes).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

pub fn parse_desktop(text: &str, id: &str) -> Option<AppEntry> {
    parse_desktop_any(text, id).filter(|e| e.shown)
}

/// [`parse_desktop`], keeping a `NoDisplay` entry with `shown` false: the
/// index [`scan`] builds holds those too, as programs that open files.
pub fn parse_desktop_any(text: &str, id: &str) -> Option<AppEntry> {
    let kv = desktop_entry_kv(text);
    let get = |k: &str| kv.get(k).map(|v| v.as_ref());
    if get("Type") != Some("Application") {
        return None;
    }
    if get("Hidden") == Some("true") {
        return None;
    }
    if let Some(only) = get("OnlyShowIn")
        && !list_contains(only, DESKTOP_NAME)
    {
        return None;
    }
    if let Some(not) = get("NotShowIn")
        && list_contains(not, DESKTOP_NAME)
    {
        return None;
    }
    let name = get("Name")?.to_string();
    let exec_line = get("Exec")?.to_string();
    if exec_argv(&exec_line).is_empty() {
        return None;
    }
    Some(AppEntry {
        name,
        exec_line,
        terminal: get("Terminal") == Some("true"),
        id: id.to_string(),
        icon: get("Icon").filter(|s| !s.is_empty()).map(str::to_string),
        startup_wm_class: get("StartupWMClass")
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        categories: split_list(get("Categories")),
        mime_types: split_list(get("MimeType")),
        shown: get("NoDisplay") != Some("true"),
    })
}

pub fn desktop_entry_kv(text: &str) -> HashMap<&str, Cow<'_, str>> {
    let mut in_entry = false;
    let mut kv: HashMap<&str, Cow<'_, str>> = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            kv.entry(k.trim())
                .or_insert_with(|| unescape_value(v.trim()));
        }
    }
    kv
}

/// A string value with the spec's escapes decoded: `\s`, `\n`, `\t`,
/// `\r` and `\\`. Any other backslash pair stays as written: `\;` is
/// the list syntax's, and older files put a lone `\"` in `Exec`.
fn unescape_value(v: &str) -> Cow<'_, str> {
    if !v.contains('\\') {
        return Cow::Borrowed(v);
    }
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(n) => {
                out.push('\\');
                out.push(n);
            }
            None => out.push('\\'),
        }
    }
    Cow::Owned(out)
}

pub fn escape_value(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

/// `arg` as one argument of a command line: bare when every character is
/// plain to both the spec's word splitting and `sh`, else in double
/// quotes with `"`, `` ` ``, `$` and `\\` backslash-escaped. `exec_argv`
/// and `sh -c` both read that form back to `arg`.
pub fn exec_arg(arg: &str) -> String {
    let plain = |c: char| c.is_alphanumeric() || "/._-+,:@%=".contains(c);
    if !arg.is_empty() && arg.chars().all(plain) {
        return arg.to_string();
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    for c in arg.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// Command line `cmd` as an `Exec=` value: `%` doubled so nothing reads as
/// a field code, then string-escaped. None when `cmd` holds a control
/// character: the spec allows none in a string, and dropping it would
/// run a different command.
pub fn exec_value(cmd: &str) -> Option<String> {
    if cmd.contains(char::is_control) {
        return None;
    }
    Some(escape_value(&cmd.replace('%', "%%")))
}

#[derive(Clone, Copy, Debug)]
pub enum Launch<'a> {
    Exec(&'a str),
    Action(&'a str),
}

/// The `.desktop` text of a launcher (shortcut or Quick Launch button).
/// Control characters are dropped from `Name` and `Icon` so a label cannot
/// add a key; a command holding one gives None (see [`exec_value`]).
pub fn launcher_text(label: &str, icon: Option<&str>, launch: Launch) -> Option<String> {
    let mut s = format!(
        "[Desktop Entry]\nType=Application\nName={}\n",
        label_value(label)
    );
    if let Some(icon) = icon {
        s.push_str(&format!("Icon={}\n", label_value(icon)));
    }
    match launch {
        Launch::Exec(cmd) => s.push_str(&format!("Exec={}\n", exec_value(cmd)?)),
        Launch::Action(name) => s.push_str(&format!("X-Reprise-Action={}\n", label_value(name))),
    }
    Some(s)
}

/// The `.desktop` text of a web link: `Type=Link`, shown as `label`,
/// opening `url`. None when `url` holds a control character, which
/// could add a key.
pub fn link_text(label: &str, url: &str) -> Option<String> {
    if url.contains(char::is_control) {
        return None;
    }
    Some(format!(
        "[Desktop Entry]\nType=Link\nName={}\nIcon=text-html\nURL={}\n",
        label_value(label),
        escape_value(url)
    ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkEntry {
    pub name: String,
    pub url: String,
    pub icon: Option<String>,
}

pub fn parse_link(text: &str) -> Option<LinkEntry> {
    let kv = desktop_entry_kv(text);
    let get = |k: &str| kv.get(k).map(|v| v.as_ref()).filter(|v| !v.is_empty());
    if get("Type") != Some("Link") {
        return None;
    }
    // Only a web page opens from a link: a `file:` or a program's own
    // scheme would hand xdg-open something other than a browser.
    let url = get("URL").filter(|u| crate::dropdata::is_web_url(u))?;
    Some(LinkEntry {
        name: get("Name")?.to_string(),
        url: url.to_string(),
        icon: get("Icon").map(str::to_string),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shortcut {
    App(AppEntry),
    Link(LinkEntry),
}

impl Shortcut {
    pub fn name(&self) -> &str {
        match self {
            Shortcut::App(a) => &a.name,
            Shortcut::Link(l) => &l.name,
        }
    }

    pub fn icon(&self) -> &str {
        match self {
            Shortcut::App(a) => a.icon.as_deref().unwrap_or("application-x-executable"),
            Shortcut::Link(l) => l.icon.as_deref().unwrap_or("text-html"),
        }
    }
}

/// The shortcut the file at `path` is (`meta` followed through a symlink);
/// None for a file that shows as itself. Only a regular, executable,
/// [trusted](is_trusted) `.desktop` file is read: an archive restores the
/// execute bit but not the marker. One that cannot be read, or holds
/// neither a program nor a web link, is none, so it can still be deleted
/// or renamed.
pub fn shortcut_at(path: &Path, meta: &std::fs::Metadata) -> Option<Shortcut> {
    use std::os::unix::fs::PermissionsExt;
    let file_name = path.file_name()?.to_string_lossy();
    if !file_name.ends_with(".desktop")
        || !meta.is_file()
        || meta.permissions().mode() & 0o111 == 0
        || !is_trusted(path)
    {
        return None;
    }
    let text = read_desktop_file(path)
        .map_err(|e| log::warn!("{}: {e}", path.display()))
        .ok()?;
    parse_desktop(&text, &file_name)
        .map(Shortcut::App)
        .or_else(|| parse_link(&text).map(Shortcut::Link))
}

/// Write launcher `text` to a new file at `path`, mode 0755 and
/// [trusted](is_trusted), as the desktop shows a launcher only while both.
/// An existing `path` is `AlreadyExists`; a file system without `user.`
/// xattrs fails and leaves no file behind.
pub fn write_launcher(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o755)
        .open(path)?;
    let written = file
        .write_all(text.as_bytes())
        .and_then(|()| set_trusted_fd(&file));
    if written.is_err() {
        let _ = std::fs::remove_file(path);
    }
    written
}

/// The desktop entry for `program`, a program file picked by path (Open
/// With's Browse...), so a file type can have it as default:
/// `reprise-userapp-<file name>.desktop` in the user's applications folder.
/// `NoDisplay=true` keeps it out of the menus, though Open With lists it
/// for its types ([`open_with`]). An existing entry for the same program is
/// reused, `mime` added to its types.
pub fn program_entry(program: &Path, mime: Option<&str>) -> std::io::Result<AppEntry> {
    let dir = user_applications_dir().ok_or_else(|| std::io::Error::other("no home directory"))?;
    program_entry_in(&dir, program, mime)
}

/// [`program_entry`] in applications folder `dir`. A path that is not
/// absolute UTF-8, or holds a control character, is refused. A name
/// taken by an entry for another program gets `-2`, `-3`, ... after it.
pub fn program_entry_in(
    dir: &Path,
    program: &Path,
    mime: Option<&str>,
) -> std::io::Result<AppEntry> {
    let refuse = |why: &str| std::io::Error::new(std::io::ErrorKind::InvalidInput, why.to_string());
    let prog = program
        .to_str()
        .filter(|_| program.is_absolute())
        .ok_or_else(|| refuse("not an absolute UTF-8 path"))?;
    let exec = exec_value(&exec_arg(prog)).ok_or_else(|| refuse("a control character"))?;
    let name = program
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let entry_text = |mimes: &[String]| {
        let mut text = format!(
            "[Desktop Entry]\nType=Application\nName={}\nIcon=application-x-executable\n\
             Exec={exec} %f\nNoDisplay=true\n",
            label_value(&name)
        );
        if !mimes.is_empty() {
            text.push_str(&format!("MimeType={};\n", mimes.join(";")));
        }
        text
    };
    let mime = mime
        .filter(|m| m.chars().all(|c| c.is_ascii_graphic() && c != ';'))
        .map(str::to_string);
    let text = entry_text(mime.as_slice());
    let stem: String = name
        .chars()
        .map(|c| match c {
            c if c.is_ascii_alphanumeric() || "._-".contains(c) => c,
            _ => '_',
        })
        .collect();
    std::fs::create_dir_all(dir)?;
    for n in 1..100 {
        let id = match n {
            1 => format!("{BROWSED_PREFIX}{stem}.desktop"),
            n => format!("{BROWSED_PREFIX}{stem}-{n}.desktop"),
        };
        let fresh =
            parse_desktop_any(&text, &id).ok_or_else(|| refuse("an entry that does not parse"))?;
        let path = dir.join(&id);
        match read_desktop_file(&path) {
            Ok(old) => {
                if let Some(e) =
                    parse_desktop_any(&old, &id).filter(|e| e.exec_line == fresh.exec_line)
                {
                    let Some(m) = mime.filter(|m| !e.mime_types.contains(m)) else {
                        return Ok(e);
                    };
                    let mut mimes = e.mime_types;
                    mimes.push(m);
                    let text = entry_text(&mimes);
                    crate::ops::write_atomic(&path, text.as_bytes())?;
                    return parse_desktop_any(&text, &id)
                        .ok_or_else(|| refuse("an entry that does not parse"));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                crate::ops::write_atomic(&path, text.as_bytes())?;
                return Ok(fresh);
            }
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other(
        "every entry name for the program is taken",
    ))
}

pub(crate) fn label_value(v: &str) -> String {
    escape_value(&v.replace(char::is_control, ""))
}

/// Launcher text `text` renamed to `name`: the `[Desktop Entry]` group's
/// first `Name=` takes `name` (see [`label_value`]); any other `Name=` and
/// every `Name[xx]=` in that group go. Other lines stay byte for byte.
pub fn with_name(text: &str, name: &str) -> String {
    let mut out = String::with_capacity(text.len() + name.len());
    let mut in_entry = false;
    let mut done = false;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let trimmed = body.trim();
        if trimmed.starts_with('[') {
            in_entry = trimmed == "[Desktop Entry]";
        } else if in_entry && !trimmed.starts_with('#') {
            let key = trimmed.split_once('=').map(|(k, _)| k.trim());
            if key.is_some_and(|k| k.starts_with("Name[")) {
                continue;
            }
            if key == Some("Name") {
                if !done {
                    done = true;
                    out.push_str("Name=");
                    out.push_str(&label_value(name));
                    out.push_str(&line[body.len()..]);
                }
                continue;
            }
        }
        out.push_str(line);
    }
    out
}

pub fn set_launcher_name(path: &Path, name: &str) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    if name.replace(char::is_control, "").trim().is_empty() {
        return Err(Error::new(ErrorKind::InvalidInput, "empty name"));
    }
    rewrite_launcher(path, &with_name(&read_desktop_file(path)?, name))
}

/// Replace the launcher at `path` with `text`: a temp file beside it takes
/// the mode and trust marker, then replaces it in one rename. A symlinked
/// launcher becomes a copy of its target holding `text`.
pub(crate) fn rewrite_launcher(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind, Write};
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)?.permissions().mode() & 0o7777;
    let trusted = is_trusted(path);
    if path.file_name().is_none() {
        return Err(Error::new(ErrorKind::InvalidInput, "no file name"));
    }
    let (mut file, tmp) =
        crate::ops::create_temp_beside(path, Some(std::fs::Permissions::from_mode(mode)))?;
    let written = (|| {
        file.write_all(text.as_bytes())?;
        if trusted {
            set_trusted_fd(&file)?;
        }
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

fn opener(is_dir: bool) -> &'static str {
    if is_dir {
        "reprise-explorer"
    } else {
        "xdg-open"
    }
}

pub const DEFAULT_TERMINAL: &str = "reprise-terminal";

/// None when `path` is not UTF-8, which no `Exec=` line can hold.
pub fn open_command_for(path: &Path) -> Option<String> {
    open_command_as(path, path.is_dir())
}

/// [`open_command_for`] for a path already known to be a folder or not.
pub fn open_command_as(path: &Path, is_dir: bool) -> Option<String> {
    Some(command_line(&[opener(is_dir), path.to_str()?]))
}

pub fn command_line<S: AsRef<str>>(argv: &[S]) -> String {
    let quoted: Vec<String> = argv.iter().map(|a| exec_arg(a.as_ref())).collect();
    quoted.join(" ")
}

pub fn open_argv_for(path: &Path) -> Vec<OsString> {
    vec![opener(path.is_dir()).into(), path.into()]
}

/// Explorer on `dir`, for a caller that already knows it is a folder.
pub fn open_folder_argv(dir: &Path) -> Vec<OsString> {
    vec!["reprise-explorer".into(), dir.into()]
}

/// Explorer on the folder `path` sits in, with `path` selected.
pub fn open_location_argv(path: &Path) -> Vec<OsString> {
    vec!["reprise-explorer".into(), "--select".into(), path.into()]
}

pub fn split_list(v: Option<&str>) -> Vec<String> {
    v.map(|v| {
        v.split(';')
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

/// The MIME type of `path` for Open With: `inode/directory` for a folder,
/// otherwise `db`'s glob match on its name, or None when nothing matches.
pub fn mime_of_in(db: &MimeDb, path: &Path) -> Option<String> {
    if path.is_dir() {
        return Some("inode/directory".into());
    }
    let name = path.file_name()?.to_string_lossy();
    db.mime_of_name(&name)
}

/// Read and parse the shared-mime-info `globs2` files that exist, the
/// user's first. Every call reads them again, so a caller keeps the result
/// (or a [`MimeGlobs`]).
pub fn mime_globs() -> MimeDb {
    read_globs(&globs2_paths())
}

fn globs2_paths() -> Vec<PathBuf> {
    mime_dirs().into_iter().map(|d| d.join("globs2")).collect()
}

pub(crate) fn mime_dirs() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    {
        roots.push(home.join("mime"));
    }
    roots.push(PathBuf::from("/usr/share/mime"));
    roots.push(PathBuf::from("/usr/local/share/mime"));
    roots
}

fn read_globs(paths: &[PathBuf]) -> MimeDb {
    MimeDb::from_texts(paths.iter().filter_map(|p| std::fs::read_to_string(p).ok()))
}

/// A [`MimeDb`] for a long-lived process: `current` stats the `globs2`
/// files and parses them again only when one appeared, went away or got a
/// new mtime (`update-mime-database` replaces them).
#[derive(Debug)]
pub struct MimeGlobs {
    paths: Vec<PathBuf>,
    /// Each path's mtime at the last parse (None: missing); None before the first parse.
    stamps: Option<Vec<Option<std::time::SystemTime>>>,
    db: MimeDb,
}

impl MimeGlobs {
    pub fn new() -> MimeGlobs {
        MimeGlobs::at(globs2_paths())
    }

    pub fn at(paths: Vec<PathBuf>) -> MimeGlobs {
        MimeGlobs {
            paths,
            stamps: None,
            db: MimeDb::empty(),
        }
    }

    pub fn current(&mut self) -> &MimeDb {
        // Stat before reading: a write in between leaves an old stamp, so
        // the next call parses again.
        let stamps: Vec<_> = self
            .paths
            .iter()
            .map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
            .collect();
        if self.stamps.as_ref() != Some(&stamps) {
            self.db = read_globs(&self.paths);
            self.stamps = Some(stamps);
        }
        &self.db
    }
}

impl Default for MimeGlobs {
    fn default() -> MimeGlobs {
        MimeGlobs::new()
    }
}

/// A byte range of a [`GlobTable`]'s `text`.
#[derive(Clone, Copy, Debug)]
struct Span {
    start: u32,
    len: u32,
}

#[derive(Clone, Copy, Debug)]
struct GlobRule {
    weight: u32,
    /// Line order within its file: the earlier of two equal matches wins.
    order: u32,
    mime: Span,
    /// Lower-cased unless case-sensitive.
    pattern: Span,
    suffix: bool,
    case_sensitive: bool,
}

/// One `globs2` file's rules, indexed so a lookup reads only the rules
/// that can match: a name's own literal rules, the suffix rules ending in
/// its last extension, and the few suffixes with no dot (`*~`).
#[derive(Debug, Default)]
struct GlobTable {
    /// The file's text followed by the lower-cased patterns that differ
    /// from their line; every [`Span`] points in here.
    text: String,
    rules: Vec<GlobRule>,
    /// `(key hash, rule index)` sorted by hash: literal names keyed by the
    /// name, dotted suffixes by what follows their last dot (`.tar.gz`
    /// under `gz`). A hash collision only adds a candidate `hits` drops.
    names: Vec<(u64, u32)>,
    by_ext: Vec<(u64, u32)>,
    other: Vec<u32>,
}

fn key_hash(key: &str) -> u64 {
    use std::hash::BuildHasher;
    rustc_hash::FxBuildHasher.hash_one(key)
}

fn rules_under(index: &[(u64, u32)], key: &str) -> impl Iterator<Item = u32> {
    let h = key_hash(key);
    let from = index.partition_point(|e| e.0 < h);
    index[from..]
        .iter()
        .take_while(move |e| e.0 == h)
        .map(|e| e.1)
}

impl GlobTable {
    /// Parse one `globs2` text. Comment lines take no line order; globs
    /// with a wildcard past a leading `*` are left out.
    fn parse(text: String) -> GlobTable {
        let mut lowered = String::new();
        let mut rules = Vec::new();
        let span = |s: &str| Span {
            start: (s.as_ptr() as usize - text.as_ptr() as usize) as u32,
            len: s.len() as u32,
        };
        let lines = text.lines().filter(|l| !l.starts_with('#'));
        for (order, line) in lines.enumerate() {
            let mut parts = line.splitn(4, ':');
            let (Some(w), Some(mime), Some(glob)) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let Ok(weight) = w.parse::<u32>() else {
                continue;
            };
            let case_sensitive = parts
                .next()
                .is_some_and(|f| f.split(',').any(|f| f == "cs"));
            let (suffix, body) = match glob.strip_prefix('*') {
                Some(s) => (true, s),
                None => (false, glob),
            };
            if body.contains(['*', '?', '[']) {
                continue;
            }
            let unchanged = case_sensitive
                || if body.is_ascii() {
                    !body.bytes().any(|b| b.is_ascii_uppercase())
                } else {
                    body.to_lowercase() == body
                };
            let pattern = if unchanged {
                span(body)
            } else {
                let start = text.len() + lowered.len();
                lowered.push_str(&body.to_lowercase());
                Span {
                    start: start as u32,
                    len: (text.len() + lowered.len() - start) as u32,
                }
            };
            rules.push(GlobRule {
                weight,
                order: order as u32,
                mime: span(mime),
                pattern,
                suffix,
                case_sensitive,
            });
        }
        let mut t = GlobTable {
            text,
            rules,
            ..GlobTable::default()
        };
        t.text.push_str(&lowered);
        for (i, r) in t.rules.iter().enumerate() {
            let i = i as u32;
            let pattern = &t.text[r.pattern.start as usize..][..r.pattern.len as usize];
            match pattern.rsplit_once('.') {
                _ if !r.suffix => t.names.push((key_hash(pattern), i)),
                Some((_, ext)) => t.by_ext.push((key_hash(ext), i)),
                None => t.other.push(i),
            }
        }
        t.names.sort_unstable();
        t.by_ext.sort_unstable();
        t
    }

    fn str(&self, s: Span) -> &str {
        &self.text[s.start as usize..][..s.len as usize]
    }

    fn hits(&self, r: &GlobRule, name: &str, lower: &str) -> bool {
        let subject = if r.case_sensitive { name } else { lower };
        let pattern = self.str(r.pattern);
        if r.suffix {
            subject.ends_with(pattern)
        } else {
            subject == pattern
        }
    }

    fn candidates(&self, subject: &str) -> impl Iterator<Item = &GlobRule> {
        let ext = subject.rsplit_once('.').map(|(_, e)| e);
        let names = rules_under(&self.names, subject);
        let by_ext = ext
            .map(|e| rules_under(&self.by_ext, e))
            .into_iter()
            .flatten();
        names.chain(by_ext).map(|i| &self.rules[i as usize])
    }
}

#[derive(Debug, Default)]
pub struct MimeDb {
    /// One list per `globs2` file, in lookup order; the first file with a
    /// hit answers, as the spec's per-directory precedence says.
    tables: Vec<GlobTable>,
    descriptions: std::sync::Mutex<HashMap<String, Option<String>>>,
}

impl MimeDb {
    /// Parse `globs2` texts, the first taking precedence.
    pub fn parse<'a>(texts: impl IntoIterator<Item = &'a str>) -> MimeDb {
        MimeDb::from_texts(texts.into_iter().map(str::to_string))
    }

    fn from_texts(texts: impl IntoIterator<Item = String>) -> MimeDb {
        MimeDb {
            tables: texts.into_iter().map(GlobTable::parse).collect(),
            descriptions: Default::default(),
        }
    }

    pub fn empty() -> MimeDb {
        MimeDb::default()
    }

    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    /// The type of a file called `name`: the highest weight wins, among
    /// equal weights the longest pattern, and then the earlier line.
    pub fn mime_of_name(&self, name: &str) -> Option<String> {
        let lower = name.to_lowercase();
        self.tables.iter().find_map(|table| {
            let cased = (lower != name).then(|| table.candidates(name));
            table
                .candidates(&lower)
                .chain(cased.into_iter().flatten())
                .chain(table.other.iter().map(|&i| &table.rules[i as usize]))
                .filter(|r| table.hits(r, name, &lower))
                .max_by_key(|r| (r.weight, r.pattern.len, std::cmp::Reverse(r.order)))
                .map(|r| table.str(r.mime).to_string())
        })
    }

    pub fn description(&self, mime: &str) -> Option<String> {
        let memo = || lock(&self.descriptions);
        if let Some(d) = memo().get(mime) {
            return d.clone();
        }
        // Read unlocked: two threads missing the same type both read it.
        let d = crate::props::mime_description(mime);
        memo().insert(mime.to_string(), d.clone());
        d
    }

    /// Each type's extension, as a file list names it (`.txt`): its
    /// heaviest `*.ext` glob, the earlier file and line among equals. A
    /// type with only literal names or wildcards has none.
    pub fn extensions(&self) -> HashMap<String, String> {
        let mut best: HashMap<&str, (u32, usize, u32, &str)> = HashMap::new();
        for (t, table) in self.tables.iter().enumerate() {
            for &(_, i) in &table.by_ext {
                let r = &table.rules[i as usize];
                let key = (r.weight, usize::MAX - t, u32::MAX - r.order);
                let mime = table.str(r.mime);
                let better = best.get(mime).is_none_or(|b| key > (b.0, b.1, b.2));
                if better {
                    best.insert(mime, (key.0, key.1, key.2, table.str(r.pattern)));
                }
            }
        }
        best.into_iter()
            .map(|(m, b)| (m.to_string(), b.3.to_string()))
            .collect()
    }
}

pub fn mime_from_globs(text: &str, name: &str) -> Option<String> {
    MimeDb::parse([text]).mime_of_name(name)
}

/// Indices into `apps` of the shown programs that open `mime`, by name. A
/// `text/*` type also takes the `text/plain` editors. A Browse... entry
/// ([`AppEntry::is_browsed`]) counts as shown here.
pub fn open_with(apps: &[AppEntry], mime: &str) -> Vec<usize> {
    let text = mime.starts_with("text/") && mime != "text/plain";
    let mut hits: Vec<usize> = apps
        .iter()
        .enumerate()
        .filter(|(_, a)| {
            (a.shown || a.is_browsed())
                && a.mime_types
                    .iter()
                    .any(|m| m == mime || (text && m == "text/plain"))
        })
        .map(|(i, _)| i)
        .collect();
    hits.sort_by_key(|&i| apps[i].name.to_lowercase());
    hits
}

/// The Open With chooser's two groups for `mime`: the programs that
/// declare it and the type's `default` program, by name, then every other
/// shown program by name, case folded. Without a type nothing is
/// recommended.
pub fn chooser_groups(
    apps: &[AppEntry],
    mime: Option<&str>,
    default: Option<usize>,
) -> (Vec<usize>, Vec<usize>) {
    let mut recommended: Vec<usize> = mime.map(|m| open_with(apps, m)).unwrap_or_default();
    if let Some(d) = default.filter(|d| mime.is_some() && !recommended.contains(d)) {
        recommended.push(d);
        recommended.sort_by_key(|&i| apps[i].name.to_lowercase());
    }
    let mut others: Vec<usize> = (0..apps.len())
        .filter(|&i| apps[i].shown && !recommended.contains(&i))
        .collect();
    others.sort_by_key(|&i| apps[i].name.to_lowercase());
    (recommended, others)
}

/// The argument vector that opens `path` with `entry`, run with no shell:
/// the entry's `Exec` split by [`exec_argv`]'s rules, with the path's own
/// bytes in the first `%f`/`%F` and its `file://` URI in a `%u`/`%U`. An
/// entry with neither code takes no file, as the spec says. A
/// `Terminal=true` entry runs under [`in_terminal`].
pub fn open_argv(entry: &AppEntry, terminal: &str, path: &Path) -> Vec<OsString> {
    // The spec allows one file code per line; any after the first drop.
    let mut used = false;
    let argv = split_exec(&entry.exec_line, |code| {
        let v = match code {
            'f' | 'F' | 'u' | 'U' if used => return Vec::new(),
            'f' | 'F' => path.as_os_str().to_owned(),
            'u' | 'U' => format!("file://{}", crate::trash::percent_encode(path)).into(),
            _ => return entry_field(entry, code),
        };
        used = true;
        vec![v]
    });
    wrap(entry, terminal, argv)
}

pub fn launch_argv(entry: &AppEntry, terminal: &str) -> Vec<OsString> {
    wrap(
        entry,
        terminal,
        split_exec(&entry.exec_line, |code| entry_field(entry, code)),
    )
}

/// What the field code `code` stands for that names `entry` itself: `%i` is `--icon <Icon>`
/// (nothing without an icon), `%c` the name. Any other code drops.
fn entry_field(entry: &AppEntry, code: char) -> Vec<OsString> {
    match code {
        'i' => entry
            .icon
            .iter()
            .filter(|i| !i.is_empty())
            .flat_map(|i| ["--icon".into(), i.into()])
            .collect(),
        'c' => vec![entry.name.clone().into()],
        _ => Vec::new(),
    }
}

fn wrap(entry: &AppEntry, terminal: &str, argv: Vec<OsString>) -> Vec<OsString> {
    if entry.terminal {
        in_terminal(terminal, argv)
    } else {
        argv
    }
}

pub fn in_terminal(terminal: &str, mut argv: Vec<OsString>) -> Vec<OsString> {
    let mut wrapped = split_exec(terminal, |_| Vec::new());
    wrapped.push("-e".into());
    wrapped.append(&mut argv);
    wrapped
}

pub fn default_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = user_applications_dir().into_iter().collect();
    let data_dirs =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    for d in data_dirs.split(':').filter(|s| !s.is_empty()) {
        dirs.push(PathBuf::from(d).join("applications"));
    }
    dirs
}

pub fn user_applications_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .map(|d| d.join("applications"))
}

pub fn find_file(id: &str) -> Option<PathBuf> {
    default_dirs()
        .into_iter()
        .map(|d| d.join(id))
        .find(|p| p.is_file())
}

/// Earlier directories override later ones by file name; result sorted by
/// name, case-insensitive. An excluded entry in an earlier directory still
/// shadows the same id in later directories, per the desktop-entry spec.
pub fn scan(dirs: &[PathBuf]) -> Vec<AppEntry> {
    let mut seen: HashMap<String, Option<AppEntry>> = HashMap::new();
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            let Some(id) = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            if !id.ends_with(".desktop") || seen.contains_key(&id) {
                continue;
            }
            let Ok(text) = read_desktop_file(&path) else {
                continue;
            };
            let app = parse_desktop_any(&text, &id);
            seen.insert(id, app);
        }
    }
    let mut list: Vec<AppEntry> = seen.into_values().flatten().collect();
    // Apps sharing a name break on their id: HashMap order changes from
    // run to run.
    list.sort_by_cached_key(|a| (a.name.to_lowercase(), a.id.clone()));
    list
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub apps: Vec<usize>,
}

/// Freedesktop main categories in filing priority, with their folder names.
/// An application goes into the first folder here that its `Categories`
/// mention, so a "Network;Utility" browser lands in Internet, not
/// Accessories. Anything unmatched goes to "Other".
const GROUPS: &[(&str, &str)] = &[
    ("Game", "Games"),
    ("AudioVideo", "Sound and Video"),
    ("Audio", "Sound and Video"),
    ("Video", "Sound and Video"),
    ("Graphics", "Graphics"),
    ("Network", "Internet"),
    ("Office", "Office"),
    ("Development", "Programming"),
    ("Education", "Education"),
    ("Science", "Science"),
    ("Settings", "Settings"),
    ("System", "System Tools"),
    ("Utility", "Accessories"),
];

const OTHER: &str = "Other";

pub fn group(apps: &[AppEntry]) -> Vec<Group> {
    let mut folders: Vec<Group> = Vec::new();
    for (i, app) in apps.iter().enumerate().filter(|(_, a)| a.shown) {
        let name = GROUPS
            .iter()
            .find(|(cat, _)| app.in_category(cat))
            .map(|(_, name)| *name)
            .unwrap_or(OTHER);
        match folders.iter_mut().find(|g| g.name == name) {
            Some(g) => g.apps.push(i),
            None => folders.push(Group {
                name: name.into(),
                apps: vec![i],
            }),
        }
    }
    folders.sort_by_key(|g| (g.name == OTHER, g.name.clone()));
    folders
}

/// Icon name for a window's app_id: the entry whose id (minus `.desktop`)
/// equals it, else the entry whose `StartupWMClass` equals it, both
/// case-insensitive. Entries without an icon never match.
pub fn icon_for_app_id<'a>(entries: &'a [AppEntry], app_id: &str) -> Option<&'a str> {
    entry_for_app_id(entries, app_id, |e| e.icon.is_some()).and_then(|e| e.icon.as_deref())
}

/// Matches the id without `.desktop`, else `StartupWMClass`, both case-insensitively.
pub fn entry_for_app_id<'a>(
    entries: &'a [AppEntry],
    app_id: &str,
    want: impl Fn(&AppEntry) -> bool,
) -> Option<&'a AppEntry> {
    if app_id.is_empty() {
        return None;
    }
    let want_id = app_id.to_lowercase();
    let by_id = entries.iter().find(|e| {
        want(e)
            && e.id
                .strip_suffix(".desktop")
                .unwrap_or(&e.id)
                .to_lowercase()
                == want_id
    });
    by_id.or_else(|| {
        entries.iter().find(|e| {
            want(e)
                && e.startup_wm_class
                    .as_deref()
                    .is_some_and(|c| c.to_lowercase() == want_id)
        })
    })
}

mod trust;
pub(crate) use trust::{TRUST_XATTR, set_trusted_fd, verifies};
pub use trust::{allow_launching, is_trusted, set_trusted};

#[cfg(test)]
mod tests;
