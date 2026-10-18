//! Application entries from freedesktop .desktop files: the Start menu,
//! Open With, and a window's icon by its app_id (the share picker).

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
    split_exec(exec, |_| None)
        .into_iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

/// [`exec_argv`]'s splitting, with field code `%c` replaced by
/// `field('c')` inside its argument; None drops it, and an argument that
/// was only the code goes with it.
fn split_exec(exec: &str, mut field: impl FnMut(char) -> Option<OsString>) -> Vec<OsString> {
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
                    if let Some(v) = field(code) {
                        arg.push(v);
                        have = true;
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

/// The xattr marking a `.desktop` file as one the user allowed to run.
/// Extraction does not restore `user.` xattrs by default, so an archive can
/// set a launcher's execute bit but not this.
const TRUST_XATTR: &std::ffi::CStr = c"user.reprise.trusted";
const TRUST_VALUE: &[u8] = b"true";

pub fn is_trusted(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mut buf = [0u8; TRUST_VALUE.len()];
    // SAFETY: `c` and the attribute name are NUL-terminated and `buf` is
    // writable for its length, all live across the call.
    let n = unsafe {
        libc::getxattr(
            c.as_ptr(),
            TRUST_XATTR.as_ptr(),
            buf.as_mut_ptr().cast(),
            buf.len(),
        )
    };
    n == TRUST_VALUE.len() as isize && buf == TRUST_VALUE
}

pub fn set_trusted(path: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "path holds a NUL"))?;
    // SAFETY: `c` and the name are NUL-terminated and the value is
    // readable for its length, all live across the call.
    let rc = unsafe {
        libc::setxattr(
            c.as_ptr(),
            TRUST_XATTR.as_ptr(),
            TRUST_VALUE.as_ptr().cast(),
            TRUST_VALUE.len(),
            0,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

pub fn allow_launching(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)?.permissions().mode();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode | 0o111))?;
    set_trusted(path)
}

pub(crate) fn set_trusted_fd(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: the fd is open for the call, the name is NUL-terminated and
    // the value is readable for its length.
    let rc = unsafe {
        libc::fsetxattr(
            file.as_raw_fd(),
            TRUST_XATTR.as_ptr(),
            TRUST_VALUE.as_ptr().cast(),
            TRUST_VALUE.len(),
            0,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
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
/// otherwise the shared-mime-info glob match on its name (the user's
/// `~/.local/share/mime/globs2` first, then the system's), or None when
/// nothing matches.
pub fn mime_of(path: &Path) -> Option<String> {
    mime_of_in(&mime_globs(), path)
}

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
    let texts: Vec<String> = paths
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect();
    MimeDb::parse(texts.iter().map(String::as_str))
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

#[derive(Clone, Debug)]
struct GlobRule {
    weight: u32,
    /// Line order within its file: the earlier of two equal matches wins.
    order: usize,
    mime: String,
    /// Lower-cased unless case-sensitive.
    pattern: String,
    suffix: bool,
    case_sensitive: bool,
}

impl GlobRule {
    fn hits(&self, name: &str, lower: &str) -> bool {
        let subject = if self.case_sensitive { name } else { lower };
        if self.suffix {
            subject.ends_with(&self.pattern)
        } else {
            subject == self.pattern
        }
    }
}

/// One `globs2` file's rules, indexed so a lookup reads only the rules
/// that can match: a name's own literal rules, the suffix rules ending in
/// its last extension, and the few suffixes with no dot (`*~`).
#[derive(Debug, Default)]
struct GlobTable {
    names: HashMap<String, Vec<GlobRule>>,
    /// Dotted suffixes, keyed by what follows their last dot (`.tar.gz` under `gz`).
    by_ext: HashMap<String, Vec<GlobRule>>,
    other: Vec<GlobRule>,
}

impl GlobTable {
    fn new(rules: Vec<GlobRule>) -> GlobTable {
        let mut t = GlobTable::default();
        for r in rules {
            match r.pattern.rsplit_once('.') {
                _ if !r.suffix => t.names.entry(r.pattern.clone()).or_default().push(r),
                Some((_, ext)) => t.by_ext.entry(ext.to_string()).or_default().push(r),
                None => t.other.push(r),
            }
        }
        t
    }

    fn candidates<'a>(&'a self, subject: &str) -> impl Iterator<Item = &'a GlobRule> {
        let ext = subject.rsplit_once('.').map(|(_, e)| e);
        let names = self.names.get(subject).into_iter().flatten();
        let by_ext = ext.and_then(|e| self.by_ext.get(e)).into_iter().flatten();
        names.chain(by_ext)
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
        MimeDb {
            tables: texts
                .into_iter()
                .map(|t| GlobTable::new(parse_globs(t)))
                .collect(),
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
                .chain(&table.other)
                .filter(|r| r.hits(name, &lower))
                .max_by_key(|r| (r.weight, r.pattern.len(), std::cmp::Reverse(r.order)))
                .map(|r| r.mime.clone())
        })
    }

    pub fn description(&self, mime: &str) -> Option<String> {
        let memo = || self.descriptions.lock().unwrap_or_else(|e| e.into_inner());
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
        let mut best: HashMap<String, (u32, usize, usize, String)> = HashMap::new();
        for (t, table) in self.tables.iter().enumerate() {
            for r in table.by_ext.values().flatten() {
                let key = (r.weight, usize::MAX - t, usize::MAX - r.order);
                let better = best.get(&r.mime).is_none_or(|b| key > (b.0, b.1, b.2));
                if better {
                    best.insert(r.mime.clone(), (key.0, key.1, key.2, r.pattern.clone()));
                }
            }
        }
        best.into_iter().map(|(m, b)| (m, b.3)).collect()
    }
}

fn parse_globs(text: &str) -> Vec<GlobRule> {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .enumerate()
        .filter_map(|(order, line)| {
            let mut parts = line.splitn(4, ':');
            let (w, mime, glob) = (parts.next()?, parts.next()?, parts.next()?);
            let weight = w.parse::<u32>().ok()?;
            let case_sensitive = parts
                .next()
                .is_some_and(|f| f.split(',').any(|f| f == "cs"));
            let (suffix, body) = match glob.strip_prefix('*') {
                Some(s) => (true, s),
                None => (false, glob),
            };
            if body.contains(['*', '?', '[']) {
                return None;
            }
            let pattern = if case_sensitive {
                body.to_string()
            } else {
                body.to_lowercase()
            };
            Some(GlobRule {
                weight,
                order,
                mime: mime.to_string(),
                pattern,
                suffix,
                case_sensitive,
            })
        })
        .collect()
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
/// bytes in the first `%f`/`%F` and its `file://` URI in a `%u`/`%U`. With
/// neither code the path goes last. A `Terminal=true` entry runs under
/// [`in_terminal`].
pub fn open_argv(entry: &AppEntry, terminal: &str, path: &Path) -> Vec<OsString> {
    // The spec allows one file code per line; any after the first drop.
    let mut used = false;
    let mut argv = split_exec(&entry.exec_line, |code| {
        let v = match code {
            _ if used => return None,
            'f' | 'F' => path.as_os_str().to_owned(),
            'u' | 'U' => format!("file://{}", crate::trash::percent_encode(path)).into(),
            _ => return None,
        };
        used = true;
        Some(v)
    });
    if !used {
        argv.push(path.into());
    }
    wrap(entry, terminal, argv)
}

pub fn launch_argv(entry: &AppEntry, terminal: &str) -> Vec<OsString> {
    wrap(entry, terminal, split_exec(&entry.exec_line, |_| None))
}

fn wrap(entry: &AppEntry, terminal: &str, argv: Vec<OsString>) -> Vec<OsString> {
    if entry.terminal {
        in_terminal(terminal, argv)
    } else {
        argv
    }
}

pub fn in_terminal(terminal: &str, mut argv: Vec<OsString>) -> Vec<OsString> {
    let mut wrapped = split_exec(terminal, |_| None);
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

#[cfg(test)]
mod tests {
    use super::*;

    const FIREFOX: &str =
        "[Desktop Entry]\nType=Application\nName=Firefox\nExec=firefox %u\nIcon=firefox\n";

    #[test]
    fn a_link_file_round_trips_and_keeps_its_keys_to_itself() {
        let text = link_text("Example\nExec=x", "https://example.org/a%20b").unwrap();
        assert_eq!(
            text,
            "[Desktop Entry]\nType=Link\nName=ExampleExec=x\nIcon=text-html\n\
             URL=https://example.org/a%20b\n"
        );
        assert_eq!(
            parse_link(&text),
            Some(LinkEntry {
                name: "ExampleExec=x".into(),
                url: "https://example.org/a%20b".into(),
                icon: Some("text-html".into()),
            })
        );
        assert_eq!(link_text("x", "https://a.test/\nExec=y"), None);
        assert_eq!(parse_link(FIREFOX), None);
        assert_eq!(parse_link("[Desktop Entry]\nType=Link\nName=x\n"), None);
        assert_eq!(
            parse_link("[Desktop Entry]\nType=Link\nName=x\nURL=file:///bin/sh\n"),
            None
        );
    }

    #[test]
    fn parses_application_and_strips_codes() {
        let e = parse_desktop(FIREFOX, "firefox.desktop").unwrap();
        assert_eq!(e.name, "Firefox");
        assert_eq!(launch_argv(&e, "foot"), ["firefox"]);
        assert!(!e.terminal);
        assert_eq!(
            strip_field_codes("app %F --flag %c \"%k\" %%"),
            "app --flag \"\" %"
        );
        assert_eq!(
            strip_field_codes("app   \"two  spaces\"  x"),
            "app \"two  spaces\" x",
            "whitespace inside quotes is the argument's"
        );
    }

    #[test]
    fn exec_argv_follows_the_spec_not_the_shell() {
        assert_eq!(exec_argv("app -a %f --b"), vec!["app", "-a", "--b"]);
        assert_eq!(
            exec_argv("\"/opt/my app/bin\" \"a \\\"quoted\\\" $HOME\" `x`;y 100%%"),
            vec!["/opt/my app/bin", "a \"quoted\" $HOME", "`x`;y", "100%"]
        );
        assert_eq!(exec_argv("  spaced   out  "), vec!["spaced", "out"]);
        assert_eq!(
            exec_argv("\"\""),
            vec![""],
            "an empty quoted argument stays"
        );
        assert!(exec_argv("").is_empty());
    }

    #[test]
    fn string_values_decode_the_spec_escapes() {
        let kv = desktop_entry_kv("[Desktop Entry]\nName=a\\sb\\tc\\\\d\\;e\\\n");
        assert_eq!(
            kv["Name"], "a b\tc\\d\\;e\\",
            "a pair it does not know stays"
        );
        for v in ["plain", "back\\slash", "line\nbreak", "tab\tcr\r", "\\s"] {
            let text = format!("[Desktop Entry]\nName={}\n", escape_value(v));
            assert_eq!(desktop_entry_kv(&text)["Name"], v, "{text}");
        }
    }

    #[test]
    fn a_hostile_label_stays_on_its_line_and_a_hostile_command_is_refused() {
        let text = launcher_text(
            "Shortcut to a\nExec=evil",
            Some("i\rcon"),
            Launch::Exec("xdg-open x"),
        )
        .unwrap();
        assert_eq!(
            text,
            "[Desktop Entry]\nType=Application\nName=Shortcut to aExec=evil\n\
             Icon=icon\nExec=xdg-open x\n"
        );
        let path = Path::new("/home/u/a\nExec=evil");
        assert_eq!(
            launcher_text("x", None, Launch::Exec(&open_command_for(path).unwrap())),
            None,
            "a command with a line break has no value that keeps it"
        );
        assert_eq!(
            launcher_text("x", None, Launch::Action("show\ndesktop")).unwrap(),
            "[Desktop Entry]\nType=Application\nName=x\nX-Reprise-Action=showdesktop\n"
        );
    }

    #[test]
    fn a_new_name_replaces_every_name_of_the_entry_and_nothing_else() {
        let text = "# keep\r\n[Desktop Entry]\r\nType=Application\r\nName[de]=Feuerfuchs\r\n\
                    Name = Firefox\r\nGenericName=Browser\r\nName=Second\r\nExec=firefox\r\n\
                    [Desktop Action new]\r\nName=New Window\r\nExec=firefox -n\r\n";
        assert_eq!(
            with_name(text, "Web\nExec=evil \\ s"),
            "# keep\r\n[Desktop Entry]\r\nType=Application\r\n\
             Name=WebExec=evil \\\\ s\r\nGenericName=Browser\r\nExec=firefox\r\n\
             [Desktop Action new]\r\nName=New Window\r\nExec=firefox -n\r\n"
        );
        assert_eq!(
            desktop_entry_kv(&with_name(FIREFOX, "a\\b"))["Name"],
            "a\\b",
            "the value reads back as typed"
        );
    }

    #[test]
    fn a_written_launcher_is_executable_and_trusted_and_a_plain_file_is_not() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("app.desktop");
        write_launcher(&path, FIREFOX).unwrap();
        assert!(is_trusted(&path));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_ne!(mode & 0o111, 0, "executable");
        let plain = t.path().join("plain.desktop");
        std::fs::write(&plain, FIREFOX).unwrap();
        assert!(!is_trusted(&plain));
        assert!(!is_trusted(&t.path().join("missing.desktop")));
    }

    #[test]
    fn renaming_a_launcher_rewrites_it_in_place_and_keeps_its_mode_and_trust() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("firefox.desktop");
        std::fs::write(&path, FIREFOX).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o750)).unwrap();
        set_trusted(&path).unwrap();
        set_launcher_name(&path, "Web").unwrap();
        assert!(is_trusted(&path), "the rewrite keeps the trust marker");
        let names: Vec<_> = std::fs::read_dir(t.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["firefox.desktop"], "no temp file is left behind");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            FIREFOX.replace("Name=Firefox", "Name=Web")
        );
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o7777, 0o750);
        assert_eq!(
            set_launcher_name(&path, "\n").unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput,
            "a name of control characters only is refused"
        );
    }

    #[test]
    fn a_shortcut_opens_the_path_it_was_made_for() {
        assert_eq!(
            launcher_text(
                "Shortcut to notes.txt",
                Some("text-plain"),
                Launch::Exec(&open_command_for(Path::new("/home/u/notes.txt")).unwrap())
            )
            .unwrap(),
            "[Desktop Entry]\nType=Application\nName=Shortcut to notes.txt\n\
             Icon=text-plain\nExec=xdg-open /home/u/notes.txt\n"
        );
        for p in [
            "/home/u/my  notes.txt",
            "/home/u/it's",
            "/home/u/$HOME `id` \"q\"",
            "/home/u/back\\slash\\\"",
            "/home/u/50% off %f",
            "/home/u/x[1]*?.txt",
            "/home/u/a;b&c|d>e",
        ] {
            let cmd = open_command_for(Path::new(p)).unwrap();
            let text = launcher_text("s", None, Launch::Exec(&cmd)).unwrap();
            let entry = parse_desktop(&text, "s.desktop").unwrap();
            assert_eq!(launch_argv(&entry, "foot"), ["xdg-open", p], "{text}");
        }
    }

    #[test]
    fn a_shortcut_to_a_name_that_is_not_utf8_is_refused() {
        use std::os::unix::ffi::OsStrExt;
        let path = Path::new(std::ffi::OsStr::from_bytes(b"/home/u/a\xffb.txt"));
        assert_eq!(open_command_for(path), None);
    }

    #[test]
    fn open_argv_for_keeps_a_name_that_is_not_utf8() {
        use std::os::unix::ffi::OsStrExt;
        let path = Path::new(std::ffi::OsStr::from_bytes(b"/nonexistent/a\xffb.txt"));
        assert_eq!(
            open_argv_for(path),
            [OsString::from("xdg-open"), path.as_os_str().to_owned()]
        );
        let dir = std::env::temp_dir();
        assert_eq!(
            open_argv_for(&dir)[0],
            "reprise-explorer",
            "a folder opens in Explorer"
        );
    }

    #[test]
    fn excludes_hidden_nodisplay_wrong_type_and_showin() {
        assert!(parse_desktop("[Desktop Entry]\nType=Link\nName=x\nExec=x\n", "a").is_none());
        let quiet = "[Desktop Entry]\nType=Application\nName=x\nExec=x\nNoDisplay=true\n";
        assert!(parse_desktop(quiet, "a").is_none());
        assert!(
            !parse_desktop_any(quiet, "a").unwrap().shown,
            "the index keeps it, unshown"
        );
        assert!(
            parse_desktop("[Desktop Entry]\nType=Application\nName=x\nExec=x\n", "a")
                .unwrap()
                .shown
        );
        assert!(
            parse_desktop(
                "[Desktop Entry]\nType=Application\nName=x\nExec=x\nHidden=true\n",
                "a"
            )
            .is_none()
        );
        assert!(
            parse_desktop(
                "[Desktop Entry]\nType=Application\nName=x\nExec=x\nOnlyShowIn=GNOME;\n",
                "a"
            )
            .is_none()
        );
        assert!(
            parse_desktop(
                "[Desktop Entry]\nType=Application\nName=x\nExec=x\nOnlyShowIn=reprise;GNOME;\n",
                "a"
            )
            .is_some()
        );
        assert!(
            parse_desktop(
                "[Desktop Entry]\nType=Application\nName=x\nExec=x\nNotShowIn=reprise;\n",
                "a"
            )
            .is_none()
        );
        assert!(parse_desktop("[Desktop Entry]\nType=Application\nName=x\n", "a").is_none());
        assert!(parse_desktop("[Other]\nType=Application\nName=x\nExec=x\n", "a").is_none());
    }

    #[test]
    fn default_terminal_is_reprise() {
        let argv = in_terminal(DEFAULT_TERMINAL, vec!["top".into()]);
        assert_eq!(argv, ["reprise-terminal", "-e", "top"]);
    }

    #[test]
    fn terminal_entries_wrap_in_terminal() {
        let e = parse_desktop(
            "[Desktop Entry]\nType=Application\nName=htop\nExec=htop\nTerminal=true\n",
            "htop.desktop",
        )
        .unwrap();
        assert!(e.terminal);
        assert_eq!(launch_argv(&e, "alacritty"), ["alacritty", "-e", "htop"]);
    }

    #[test]
    fn launch_argv_passes_shell_syntax_through_as_plain_arguments() {
        let e = parse_desktop(
            "[Desktop Entry]\nType=Application\nName=x\n\
             Exec=prog $HOME `id`; *.txt \"a b\" %U\n",
            "x.desktop",
        )
        .unwrap();
        assert_eq!(
            launch_argv(&e, "foot"),
            ["prog", "$HOME", "`id`;", "*.txt", "a b"]
        );
    }

    #[test]
    fn open_argv_fills_the_file_slot_with_the_path_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let entry = |exec: &str, terminal: bool| {
            let text = format!(
                "[Desktop Entry]\nType=Application\nName=x\nExec={exec}\nTerminal={terminal}\n"
            );
            parse_desktop(&text, "x.desktop").unwrap()
        };
        let path = Path::new(std::ffi::OsStr::from_bytes(b"/home/u/it's a\xff.txt"));
        let open = |exec, terminal, term| -> Vec<Vec<u8>> {
            open_argv(&entry(exec, terminal), term, path)
                .iter()
                .map(|a| a.as_bytes().to_vec())
                .collect()
        };
        let p = path.as_os_str().as_bytes().to_vec();
        let b = |s: &str| s.as_bytes().to_vec();
        assert_eq!(
            open("ed --new %f --x", false, ""),
            [b("ed"), b("--new"), p.clone(), b("--x")]
        );
        assert_eq!(open("ed %F", false, ""), [b("ed"), p.clone()]);
        assert_eq!(
            open("web %u", false, ""),
            [b("web"), b("file:///home/u/it%27s%20a%FF.txt")]
        );
        assert_eq!(
            open("ed %f %f", false, ""),
            [b("ed"), p.clone()],
            "one file code"
        );
        assert_eq!(
            open("\"/opt/my ed\" -i %i", false, ""),
            [b("/opt/my ed"), b("-i"), p.clone()]
        );
        assert_eq!(
            open("vim %F", true, "foot --title \"a b\""),
            [
                b("foot"),
                b("--title"),
                b("a b"),
                b("-e"),
                b("vim"),
                p.clone()
            ]
        );
    }

    #[test]
    fn parses_icon_and_startup_wm_class() {
        let e = parse_desktop(FIREFOX, "firefox.desktop").unwrap();
        assert_eq!(e.icon.as_deref(), Some("firefox"));
        assert_eq!(e.startup_wm_class, None);
        let e = parse_desktop(
            "[Desktop Entry]\nType=Application\nName=Code\nExec=code\nIcon=/opt/code/code.png\nStartupWMClass=Code\n",
            "code.desktop",
        )
        .unwrap();
        assert_eq!(e.icon.as_deref(), Some("/opt/code/code.png"));
        assert_eq!(e.startup_wm_class.as_deref(), Some("Code"));
    }

    #[test]
    fn icon_for_app_id_matches_id_then_wm_class() {
        let entries = vec![
            parse_desktop(FIREFOX, "org.mozilla.firefox.desktop").unwrap(),
            parse_desktop(
                "[Desktop Entry]\nType=Application\nName=Code\nExec=code\nIcon=vscode\nStartupWMClass=Code\n",
                "code-oss.desktop",
            )
            .unwrap(),
            parse_desktop(
                "[Desktop Entry]\nType=Application\nName=NoIcon\nExec=x\nStartupWMClass=x\n",
                "x.desktop",
            )
            .unwrap(),
        ];
        assert_eq!(
            icon_for_app_id(&entries, "org.mozilla.firefox"),
            Some("firefox")
        );
        assert_eq!(
            icon_for_app_id(&entries, "ORG.MOZILLA.FIREFOX"),
            Some("firefox")
        );
        assert_eq!(icon_for_app_id(&entries, "Code"), Some("vscode"));
        assert_eq!(icon_for_app_id(&entries, "code"), Some("vscode"));
        assert_eq!(icon_for_app_id(&entries, "x"), None);
        assert_eq!(icon_for_app_id(&entries, "nothing"), None);
        assert_eq!(icon_for_app_id(&entries, ""), None);
    }

    #[test]
    fn icon_for_app_id_prefers_the_id_match_over_a_class_match() {
        let entries = vec![
            parse_desktop(
                "[Desktop Entry]\nType=Application\nName=Wrapper\nExec=w\nIcon=wrapper\nStartupWMClass=code\n",
                "wrapper.desktop",
            )
            .unwrap(),
            parse_desktop(
                "[Desktop Entry]\nType=Application\nName=Code\nExec=code\nIcon=vscode\n",
                "code.desktop",
            )
            .unwrap(),
        ];
        assert_eq!(icon_for_app_id(&entries, "code"), Some("vscode"));
    }

    #[test]
    fn scan_orders_and_overrides() {
        let base = std::env::temp_dir().join(format!("reprise-apps-{}", std::process::id()));
        let user = base.join("user");
        let sys = base.join("sys");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&sys).unwrap();
        std::fs::write(
            sys.join("b.desktop"),
            "[Desktop Entry]\nType=Application\nName=beta\nExec=b\n",
        )
        .unwrap();
        std::fs::write(
            sys.join("a.desktop"),
            "[Desktop Entry]\nType=Application\nName=Alpha\nExec=a-sys\n",
        )
        .unwrap();
        std::fs::write(
            user.join("a.desktop"),
            "[Desktop Entry]\nType=Application\nName=alpha\nExec=a-user\n",
        )
        .unwrap();
        std::fs::write(
            sys.join("c.desktop"),
            "[Desktop Entry]\nType=Application\nName=gamma\nExec=c-sys\n",
        )
        .unwrap();
        std::fs::write(
            user.join("c.desktop"),
            "[Desktop Entry]\nType=Application\nName=gamma\nExec=c-user\nHidden=true\n",
        )
        .unwrap();
        std::fs::write(sys.join("notes.txt"), "ignored").unwrap();
        let list = scan(&[user.clone(), sys.clone()]);
        assert_eq!(
            list.iter()
                .map(|e| e.exec_line.as_str())
                .collect::<Vec<_>>(),
            vec!["a-user", "b"]
        );
        assert_eq!(list[0].name, "alpha");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn group_files_apps_by_first_matching_category() {
        let app = |name: &str, cats: &[&str]| AppEntry {
            name: name.into(),
            exec_line: name.to_lowercase(),
            terminal: false,
            id: format!("{name}.desktop"),
            icon: None,
            startup_wm_class: None,
            categories: cats.iter().map(|c| c.to_string()).collect(),
            mime_types: Vec::new(),
            shown: true,
        };
        let apps = [
            app("Zed", &["Development", "Utility"]),
            app("Firefox", &["Network", "WebBrowser"]),
            app("Calc", &["Utility"]),
            app("Mystery", &[]),
            app("Chess", &["Game", "BoardGame"]),
            app("Alacritty", &["System", "TerminalEmulator"]),
        ];
        let groups = group(&apps);
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Accessories",
                "Games",
                "Internet",
                "Programming",
                "System Tools",
                "Other"
            ]
        );
        assert_eq!(groups[3].apps, [0], "Zed: the first category wins");
        assert_eq!(groups[5].apps, [3], "Mystery");
        assert!(group(&[]).is_empty());
    }

    #[test]
    fn mime_db_answers_per_table_in_order() {
        let db = MimeDb::parse([
            "50:text/x-mine:*.txt\n",
            "50:text/plain:*.txt\n50:text/x-c:*.c:cs\n",
        ]);
        assert_eq!(db.mime_of_name("A.TXT").as_deref(), Some("text/x-mine"));
        assert_eq!(db.mime_of_name("main.c").as_deref(), Some("text/x-c"));
        assert_eq!(db.mime_of_name("MAIN.C"), None);
        assert!(MimeDb::empty().is_empty());
        assert_eq!(MimeDb::empty().mime_of_name("a.txt"), None);
    }

    #[test]
    fn mime_description_is_memoised() {
        let db = MimeDb::empty();
        let first = db.description("application/x-reprise-no-such-type");
        assert_eq!(first, None);
        assert!(
            db.descriptions
                .lock()
                .unwrap()
                .contains_key("application/x-reprise-no-such-type")
        );
    }

    #[test]
    fn mime_globs_pick_the_heaviest_longest_match() {
        let globs = "# comment\n50:text/plain:*.txt\n50:text/x-readme:readme\n\
                     80:text/html:*.htm\n50:text/html:*.html\n\
                     55:application/x-perl:*.pl\n50:text/x-c:*.c:cs\n\
                     50:text/x-makefile:makefile\n60:application/x-tar:*.tar.gz\n\
                     50:application/gzip:*.gz\n";
        assert_eq!(
            mime_from_globs(globs, "Notes.TXT").as_deref(),
            Some("text/plain")
        );
        assert_eq!(
            mime_from_globs(globs, "index.html").as_deref(),
            Some("text/html")
        );
        assert_eq!(
            mime_from_globs(globs, "README").as_deref(),
            Some("text/x-readme")
        );
        assert_eq!(
            mime_from_globs(globs, "a.tar.gz").as_deref(),
            Some("application/x-tar")
        );
        assert_eq!(
            mime_from_globs(globs, "main.c").as_deref(),
            Some("text/x-c")
        );
        assert_eq!(mime_from_globs(globs, "MAIN.C"), None);
        assert_eq!(mime_from_globs(globs, "photo.jpg"), None);
    }

    #[test]
    fn mime_globs_match_dotless_suffixes_and_break_ties_by_line_order() {
        let globs = "50:application/x-trash:*~\n50:text/x-suffix:*a.b\n\
                     50:text/x-literal:a.b\n50:text/x-upper:*.C:cs\n";
        assert_eq!(
            mime_from_globs(globs, "notes.txt~").as_deref(),
            Some("application/x-trash")
        );
        assert_eq!(
            mime_from_globs(globs, "a.b").as_deref(),
            Some("text/x-suffix"),
            "equal weight and length: the earlier line"
        );
        assert_eq!(
            mime_from_globs(globs, "Main.C").as_deref(),
            Some("text/x-upper")
        );
    }

    #[test]
    fn a_types_extension_is_its_heaviest_suffix() {
        let user = "50:text/x-c:*.cc\n";
        let system = "50:text/x-c:*.c\n60:text/plain:*.txt\n50:text/plain:*.asc\n\
                      50:image/jpeg:*.jpg\n50:image/jpeg:*.jpeg\n50:text/x-makefile:makefile\n";
        let ext = MimeDb::parse([user, system]).extensions();
        assert_eq!(ext.get("text/plain").map(String::as_str), Some(".txt"));
        assert_eq!(ext.get("image/jpeg").map(String::as_str), Some(".jpg"));
        assert_eq!(ext.get("text/x-c").map(String::as_str), Some(".cc"));
        assert_eq!(
            ext.get("text/x-makefile"),
            None,
            "a literal name is no extension"
        );
    }

    #[test]
    fn mime_globs_parse_again_only_when_a_file_changes() {
        let dir = std::env::temp_dir().join(format!("reprisefs-globs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("globs2");
        let mut globs = MimeGlobs::at(vec![dir.join("missing"), file.clone()]);
        let mime = |g: &mut MimeGlobs| g.current().mime_of_name("a.txt");
        assert_eq!(mime(&mut globs), None, "no file yet: no type");

        std::fs::write(&file, "50:text/plain:*.txt\n").unwrap();
        assert_eq!(mime(&mut globs).as_deref(), Some("text/plain"));

        let stamp = std::fs::metadata(&file).unwrap().modified().unwrap();
        std::fs::write(&file, "50:text/x-other:*.txt\n").unwrap();
        let f = std::fs::File::options().write(true).open(&file).unwrap();
        f.set_modified(stamp).unwrap();
        assert_eq!(mime(&mut globs).as_deref(), Some("text/plain"));

        f.set_modified(stamp + std::time::Duration::from_secs(1))
            .unwrap();
        assert_eq!(mime(&mut globs).as_deref(), Some("text/x-other"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_with_lists_the_handlers_by_name_and_text_editors_for_text() {
        let app = |name: &str, mimes: &[&str]| AppEntry {
            name: name.into(),
            exec_line: name.to_lowercase(),
            terminal: name == "Vim",
            id: format!("{name}.desktop"),
            icon: None,
            startup_wm_class: None,
            categories: Vec::new(),
            mime_types: mimes.iter().map(|m| m.to_string()).collect(),
            shown: true,
        };
        let apps = vec![
            app("Vim", &["text/plain", "text/x-c"]),
            app("Firefox", &["text/html", "image/png"]),
            app("Eye of GNOME", &["image/png"]),
            app("Files", &["inode/directory"]),
        ];
        assert_eq!(open_with(&apps, "image/png"), vec![2, 1]);
        assert_eq!(open_with(&apps, "text/html"), vec![1, 0]);
        assert_eq!(open_with(&apps, "text/plain"), vec![0]);
        assert_eq!(open_with(&apps, "inode/directory"), vec![3]);
        assert!(open_with(&apps, "audio/ogg").is_empty());
        assert_eq!(
            open_argv(&apps[0], "foot", Path::new("/home/u/my notes.txt")),
            ["foot", "-e", "vim", "/home/u/my notes.txt"]
        );
        assert!(
            parse_desktop(
                "[Desktop Entry]\nType=Application\nName=X\nExec=x %F\nMimeType=text/plain;;image/png;\n",
                "x.desktop"
            )
            .unwrap()
            .mime_types
                == ["text/plain", "image/png"]
        );
    }

    #[test]
    fn a_browsed_program_gets_an_unshown_entry_that_opens_its_file() {
        let dir = std::env::temp_dir().join(format!("reprisefs-userapp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let prog = Path::new("/opt/My Tools/view$1");
        let e = program_entry_in(&dir, prog, Some("text/x-python")).unwrap();
        assert_eq!(e.id, "reprise-userapp-view_1.desktop");
        assert_eq!(e.name, "view$1");
        assert!(!e.shown, "kept out of the menus");
        assert_eq!(e.icon.as_deref(), Some("application-x-executable"));
        assert_eq!(e.mime_types, ["text/x-python"]);
        assert_eq!(
            open_argv(&e, "foot", Path::new("/home/u/a b.py")),
            ["/opt/My Tools/view$1", "/home/u/a b.py"]
        );
        let text = std::fs::read_to_string(dir.join(&e.id)).unwrap();
        assert_eq!(
            parse_desktop_any(&text, &e.id),
            Some(e.clone()),
            "the file says the same"
        );
        let again = program_entry_in(&dir, prog, Some("text/plain")).unwrap();
        assert_eq!(again.id, e.id, "the same program reuses its entry");
        assert_eq!(
            again.mime_types,
            ["text/x-python", "text/plain"],
            "and adds the new type"
        );
        let text = std::fs::read_to_string(dir.join(&e.id)).unwrap();
        assert_eq!(parse_desktop_any(&text, &e.id), Some(again.clone()));
        assert_eq!(
            program_entry_in(&dir, prog, Some("text/plain")).unwrap(),
            again,
            "a type it has is not added twice"
        );
        let other = program_entry_in(&dir, Path::new("/usr/bin/view$1"), None).unwrap();
        assert_eq!(
            other.id, "reprise-userapp-view_1-2.desktop",
            "another program with the name"
        );
        assert!(other.mime_types.is_empty());
        assert_eq!(
            program_entry_in(&dir, Path::new("rel/view"), None)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unshown_entry_is_left_out_of_the_lists_unless_it_is_the_default() {
        let mut quiet = parse_desktop_any(
            "[Desktop Entry]\nType=Application\nName=Quiet\nExec=q %f\nNoDisplay=true\nMimeType=text/plain;\nCategories=Utility;\n",
            "quiet.desktop",
        )
        .unwrap();
        let loud = AppEntry {
            name: "Loud".into(),
            id: "loud.desktop".into(),
            shown: true,
            ..quiet.clone()
        };
        let apps = vec![loud, quiet.clone()];
        assert_eq!(open_with(&apps, "text/plain"), [0]);
        assert_eq!(
            chooser_groups(&apps, Some("text/plain"), None),
            (vec![0], vec![])
        );
        assert_eq!(chooser_groups(&apps, None, None), (vec![], vec![0]));
        assert_eq!(
            chooser_groups(&apps, Some("text/plain"), Some(1)),
            (vec![0, 1], vec![]),
            "the default is listed however it is marked"
        );
        assert_eq!(
            group(&apps),
            [Group {
                name: "Accessories".into(),
                apps: vec![0]
            }]
        );
        quiet.shown = true;
        assert_eq!(open_with(&[quiet], "text/plain"), [0]);
    }

    #[test]
    fn open_with_lists_a_browsed_program_for_its_types_only() {
        let dir = tempfile::tempdir().unwrap();
        let browsed =
            program_entry_in(dir.path(), Path::new("/opt/view"), Some("image/png")).unwrap();
        let quiet = parse_desktop_any(
            "[Desktop Entry]\nType=Application\nName=Quiet\nExec=q %f\nNoDisplay=true\nMimeType=image/png;\n",
            "quiet.desktop",
        )
        .unwrap();
        let shown = AppEntry {
            name: "Paint".into(),
            id: "paint.desktop".into(),
            shown: true,
            ..quiet.clone()
        };
        assert!(browsed.is_browsed() && !quiet.is_browsed());
        let apps = vec![quiet, shown, browsed];
        assert_eq!(open_with(&apps, "image/png"), [1, 2]);
        assert!(open_with(&apps, "image/jpeg").is_empty());
        assert_eq!(
            chooser_groups(&apps, Some("image/png"), None),
            (vec![1, 2], vec![]),
            "the chooser recommends it for its type"
        );
        assert_eq!(
            chooser_groups(&apps, Some("image/jpeg"), None),
            (vec![], vec![1]),
            "and leaves it out for any other"
        );
    }

    #[test]
    fn chooser_groups_put_the_type_s_programs_and_default_first_and_the_rest_by_name() {
        let e = |name: &str, id: &str, mimes: &[&str]| AppEntry {
            name: name.into(),
            exec_line: String::new(),
            terminal: false,
            id: id.into(),
            icon: None,
            startup_wm_class: None,
            categories: Vec::new(),
            mime_types: mimes.iter().map(|m| m.to_string()).collect(),
            shown: true,
        };
        let apps = vec![
            e("Zed", "zed.desktop", &[]),
            e("Notepad", "notepad.desktop", &["text/plain"]),
            e("ark", "ark.desktop", &[]),
        ];
        assert_eq!(
            chooser_groups(&apps, Some("text/plain"), None),
            (vec![1], vec![2, 0])
        );
        assert_eq!(chooser_groups(&apps, None, None), (vec![], vec![2, 1, 0]));
        assert_eq!(
            chooser_groups(&apps, Some("text/plain"), Some(0)),
            (vec![1, 0], vec![2]),
            "the default is recommended even when it does not declare the type"
        );
        assert_eq!(
            chooser_groups(&apps, Some("text/plain"), Some(1)),
            (vec![1], vec![2, 0])
        );
    }
}
